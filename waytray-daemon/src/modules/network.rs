//! Network module - displays network status and speed

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::NetworkModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// Network traffic tracking state
struct TrafficState {
    rx_bytes: u64,
    tx_bytes: u64,
}

/// Network module that displays connection status and speed
pub struct NetworkModule {
    config: RwLock<NetworkModuleConfig>,
    traffic_state: RwLock<Option<TrafficState>>,
}

impl NetworkModule {
    pub fn new(config: NetworkModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            traffic_state: RwLock::new(None),
        }
    }

    /// Get the default route interface from /proc/net/route
    async fn get_default_interface(&self) -> Option<String> {
        let content = tokio::fs::read_to_string("/proc/net/route").await.ok()?;

        // Skip header, find line with destination 00000000 (default route)
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "00000000" {
                return Some(parts[0].to_string());
            }
        }

        None
    }

    /// Get the interface to monitor (configured or auto-detected)
    async fn get_interface(&self) -> Option<String> {
        let config = self.config.read().await;
        if !config.interface.is_empty() {
            return Some(config.interface.clone());
        }
        drop(config);

        self.get_default_interface().await
    }

    /// Check if interface is up and connected
    async fn is_connected(&self, interface: &str) -> bool {
        let path = format!("/sys/class/net/{}/operstate", interface);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            let state = content.trim();
            return state == "up";
        }
        false
    }

    /// Get IP address for interface
    async fn get_ip_address(&self, interface: &str) -> Option<String> {
        // Try to read from /proc/net/fib_trie or use ip command
        // For simplicity, we'll parse ip addr output
        let output = tokio::process::Command::new("ip")
            .args(["-4", "addr", "show", interface])
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.starts_with("inet ") {
                // Format: inet 192.168.1.100/24 brd ...
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    // Remove CIDR suffix
                    let ip = parts[1].split('/').next()?;
                    return Some(ip.to_string());
                }
            }
        }

        None
    }

    /// Get network traffic bytes
    async fn get_traffic_bytes(&self, interface: &str) -> Option<(u64, u64)> {
        let rx_path = format!("/sys/class/net/{}/statistics/rx_bytes", interface);
        let tx_path = format!("/sys/class/net/{}/statistics/tx_bytes", interface);

        let rx_content = tokio::fs::read_to_string(&rx_path).await.ok()?;
        let tx_content = tokio::fs::read_to_string(&tx_path).await.ok()?;

        let rx_bytes: u64 = rx_content.trim().parse().ok()?;
        let tx_bytes: u64 = tx_content.trim().parse().ok()?;

        Some((rx_bytes, tx_bytes))
    }

    /// Calculate and update speed, returns (rx_speed_bps, tx_speed_bps)
    async fn get_speed(&self, interface: &str, interval_secs: u64) -> Option<(u64, u64)> {
        let (rx_bytes, tx_bytes) = self.get_traffic_bytes(interface).await?;

        let mut state = self.traffic_state.write().await;

        let speed = if let Some(prev) = state.as_ref() {
            let rx_delta = rx_bytes.saturating_sub(prev.rx_bytes);
            let tx_delta = tx_bytes.saturating_sub(prev.tx_bytes);

            // Convert to bytes per second
            let rx_speed = rx_delta / interval_secs;
            let tx_speed = tx_delta / interval_secs;

            Some((rx_speed, tx_speed))
        } else {
            None // First reading
        };

        *state = Some(TrafficState { rx_bytes, tx_bytes });

        speed
    }

    /// Format bytes per second to human readable string
    fn format_speed(bytes_per_sec: u64) -> String {
        if bytes_per_sec >= 1_000_000_000 {
            format!("{:.1}GB/s", bytes_per_sec as f64 / 1_000_000_000.0)
        } else if bytes_per_sec >= 1_000_000 {
            format!("{:.1}MB/s", bytes_per_sec as f64 / 1_000_000.0)
        } else if bytes_per_sec >= 1_000 {
            format!("{:.0}KB/s", bytes_per_sec as f64 / 1_000.0)
        } else {
            format!("{}B/s", bytes_per_sec)
        }
    }

    /// Get connection type icon based on interface name
    fn get_icon_for_interface(interface: &str) -> &'static str {
        if interface.starts_with("wl") || interface.starts_with("wifi") {
            "network-wireless"
        } else if interface.starts_with("eth") || interface.starts_with("en") {
            "network-wired"
        } else if interface.starts_with("tun") || interface.starts_with("tap") {
            "network-vpn"
        } else {
            "network-transmit-receive"
        }
    }

    async fn create_items(&self) -> Vec<ModuleItem> {
        let config = self.config.read().await;
        let interval = config.interval_seconds;
        let show_ip = config.show_ip;
        let show_speed = config.show_speed;
        drop(config);

        let Some(interface) = self.get_interface().await else {
            // No interface found
            return vec![ModuleItem {
                id: "network:status".to_string(),
                module: "network".to_string(),
                label: "No Network".to_string(),
                icon_name: Some("network-offline".to_string()),
                icon_pixmap: None,
                icon_width: 0,
                icon_height: 0,
                tooltip: Some("No network interface found".to_string()),
                actions: Vec::new(),
            }];
        };

        let connected = self.is_connected(&interface).await;

        if !connected {
            return vec![ModuleItem {
                id: "network:status".to_string(),
                module: "network".to_string(),
                label: "Disconnected".to_string(),
                icon_name: Some("network-offline".to_string()),
                icon_pixmap: None,
                icon_width: 0,
                icon_height: 0,
                tooltip: Some(format!("Interface {} is disconnected", interface)),
                actions: Vec::new(),
            }];
        }

        let icon = Self::get_icon_for_interface(&interface);
        let mut items = Vec::new();

        // Build label and tooltip
        let mut label_parts = Vec::new();
        let mut tooltip_parts = vec![format!("Interface: {}", interface)];

        // IP address
        if show_ip {
            if let Some(ip) = self.get_ip_address(&interface).await {
                tooltip_parts.push(format!("IP: {}", ip));
            }
        }

        // Speed
        if show_speed {
            if let Some((rx_speed, tx_speed)) = self.get_speed(&interface, interval).await {
                let rx_str = Self::format_speed(rx_speed);
                let tx_str = Self::format_speed(tx_speed);
                label_parts.push(format!("↓{} ↑{}", rx_str, tx_str));
                tooltip_parts.push(format!("Download: {}", rx_str));
                tooltip_parts.push(format!("Upload: {}", tx_str));
            } else {
                // First reading, show placeholder
                label_parts.push("↓-- ↑--".to_string());
            }
        }

        let label = if label_parts.is_empty() {
            interface.clone()
        } else {
            label_parts.join(" ")
        };

        items.push(ModuleItem {
            id: "network:status".to_string(),
            module: "network".to_string(),
            label,
            icon_name: Some(icon.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip_parts.join("\n")),
            actions: Vec::new(),
        });

        items
    }
}

#[async_trait]
impl Module for NetworkModule {
    fn name(&self) -> &str {
        "network"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Initial read to populate traffic state
        if let Some(interface) = self.get_interface().await {
            let _ = self.get_traffic_bytes(&interface).await;
        }

        // Wait a moment for first delta
        let interval = {
            let config = self.config.read().await;
            Duration::from_secs(config.interval_seconds)
        };
        tokio::time::sleep(interval).await;

        // Send initial items
        let items = self.create_items().await;
        ctx.send_items("network", items);

        // Poll at configured interval
        loop {
            let interval = {
                let config = self.config.read().await;
                Duration::from_secs(config.interval_seconds)
            };

            tokio::time::sleep(interval).await;

            let items = self.create_items().await;
            ctx.send_items("network", items);
        }
    }

    async fn stop(&self) {
        tracing::info!("Network module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // Network module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref network_config) = config.modules.network {
            let mut current = self.config.write().await;
            *current = network_config.clone();
            tracing::debug!("Network module config reloaded");
            true
        } else {
            false
        }
    }
}
