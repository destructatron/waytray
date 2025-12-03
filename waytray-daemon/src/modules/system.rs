//! System module - displays CPU and memory usage

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::SystemModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// CPU usage tracking state
struct CpuState {
    prev_idle: u64,
    prev_total: u64,
}

/// System module that displays CPU and memory usage
pub struct SystemModule {
    config: RwLock<SystemModuleConfig>,
    cpu_state: RwLock<Option<CpuState>>,
}

impl SystemModule {
    pub fn new(config: SystemModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            cpu_state: RwLock::new(None),
        }
    }

    /// Read CPU usage from /proc/stat
    /// Returns percentage (0-100)
    async fn get_cpu_usage(&self) -> Option<u8> {
        let content = tokio::fs::read_to_string("/proc/stat").await.ok()?;
        let first_line = content.lines().next()?;

        // Format: cpu user nice system idle iowait irq softirq steal guest guest_nice
        if !first_line.starts_with("cpu ") {
            return None;
        }

        let values: Vec<u64> = first_line
            .split_whitespace()
            .skip(1) // Skip "cpu"
            .filter_map(|s| s.parse().ok())
            .collect();

        if values.len() < 4 {
            return None;
        }

        let idle = values[3] + values.get(4).unwrap_or(&0); // idle + iowait
        let total: u64 = values.iter().sum();

        let mut state = self.cpu_state.write().await;

        let usage = if let Some(prev) = state.as_ref() {
            let idle_delta = idle.saturating_sub(prev.prev_idle);
            let total_delta = total.saturating_sub(prev.prev_total);

            if total_delta > 0 {
                let usage = 100.0 * (1.0 - (idle_delta as f64 / total_delta as f64));
                usage.round() as u8
            } else {
                0
            }
        } else {
            0 // First reading, no delta yet
        };

        *state = Some(CpuState {
            prev_idle: idle,
            prev_total: total,
        });

        Some(usage)
    }

    /// Read memory usage from /proc/meminfo
    /// Returns (used_percent, used_gb, total_gb)
    async fn get_memory_usage(&self) -> Option<(u8, f64, f64)> {
        let content = tokio::fs::read_to_string("/proc/meminfo").await.ok()?;

        let mut mem_total: Option<u64> = None;
        let mut mem_available: Option<u64> = None;

        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                mem_total = line.split_whitespace().nth(1)?.parse().ok();
            } else if line.starts_with("MemAvailable:") {
                mem_available = line.split_whitespace().nth(1)?.parse().ok();
            }

            if mem_total.is_some() && mem_available.is_some() {
                break;
            }
        }

        let total = mem_total?;
        let available = mem_available?;
        let used = total.saturating_sub(available);

        let percent = ((used as f64 / total as f64) * 100.0).round() as u8;
        let used_gb = used as f64 / 1_048_576.0; // kB to GB
        let total_gb = total as f64 / 1_048_576.0;

        Some((percent, used_gb, total_gb))
    }

    /// Read CPU temperature from thermal zones
    /// Returns temperature in Celsius
    async fn get_temperature(&self) -> Option<f32> {
        // Try thermal_zone0 first (most common for CPU)
        if let Ok(content) = tokio::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").await {
            if let Ok(millidegrees) = content.trim().parse::<i32>() {
                return Some(millidegrees as f32 / 1000.0);
            }
        }

        // Fallback: try to find a thermal zone with "cpu" or "x86_pkg" in its type
        if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/thermal").await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if let Some(name) = path.file_name() {
                    if name.to_string_lossy().starts_with("thermal_zone") {
                        // Check the type
                        let type_path = path.join("type");
                        if let Ok(zone_type) = tokio::fs::read_to_string(&type_path).await {
                            let zone_type = zone_type.trim().to_lowercase();
                            if zone_type.contains("cpu") || zone_type.contains("x86_pkg") || zone_type.contains("core") {
                                let temp_path = path.join("temp");
                                if let Ok(content) = tokio::fs::read_to_string(&temp_path).await {
                                    if let Ok(millidegrees) = content.trim().parse::<i32>() {
                                        return Some(millidegrees as f32 / 1000.0);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    fn create_cpu_item(&self, usage: u8) -> ModuleItem {
        // Use generic system monitor icon (cpu-specific icons often not available)
        let icon_name = "utilities-system-monitor";

        ModuleItem {
            id: "system:cpu".to_string(),
            module: "system".to_string(),
            label: format!("CPU {}%", usage),
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(format!("CPU Usage: {}%", usage)),
            actions: Vec::new(),
        }
    }

    fn create_memory_item(&self, percent: u8, used_gb: f64, total_gb: f64) -> ModuleItem {
        ModuleItem {
            id: "system:memory".to_string(),
            module: "system".to_string(),
            label: format!("Mem {}%", percent),
            icon_name: Some("drive-harddisk".to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(format!(
                "Memory: {:.1} GB / {:.1} GB ({}%)",
                used_gb, total_gb, percent
            )),
            actions: Vec::new(),
        }
    }

    fn create_temperature_item(&self, temp: f32) -> ModuleItem {
        // Choose icon based on temperature
        let icon_name = if temp >= 80.0 {
            "dialog-warning" // Hot!
        } else {
            "sensors-temperature"
        };

        ModuleItem {
            id: "system:temperature".to_string(),
            module: "system".to_string(),
            label: format!("{:.0}°C", temp),
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(format!("CPU Temperature: {:.1}°C", temp)),
            actions: Vec::new(),
        }
    }

    async fn create_items(&self) -> Vec<ModuleItem> {
        let config = self.config.read().await;
        let mut items = Vec::new();

        // CPU usage and temperature together
        if config.show_cpu {
            if let Some(usage) = self.get_cpu_usage().await {
                items.push(self.create_cpu_item(usage));
            }
        }

        if config.show_temperature {
            if let Some(temp) = self.get_temperature().await {
                items.push(self.create_temperature_item(temp));
            }
        }

        // Memory last
        if config.show_memory {
            if let Some((percent, used_gb, total_gb)) = self.get_memory_usage().await {
                items.push(self.create_memory_item(percent, used_gb, total_gb));
            }
        }

        items
    }
}

#[async_trait]
impl Module for SystemModule {
    fn name(&self) -> &str {
        "system"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Initial read to populate CPU state
        let _ = self.get_cpu_usage().await;

        // Wait a moment for first delta (cancellable)
        tokio::select! {
            _ = ctx.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }

        // Send initial items
        let items = self.create_items().await;
        ctx.send_items("system", items);

        // Poll at configured interval (re-read each iteration for hot reload)
        loop {
            let interval = {
                let config = self.config.read().await;
                Duration::from_secs(config.interval_seconds)
            };

            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(interval) => {
                    let items = self.create_items().await;
                    ctx.send_items("system", items);
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("System module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // System module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref system_config) = config.modules.system {
            let mut current = self.config.write().await;
            *current = system_config.clone();
            tracing::debug!("System module config reloaded");
            true
        } else {
            false
        }
    }
}
