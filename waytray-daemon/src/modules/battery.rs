//! Battery module - displays battery status using UPower D-Bus interface

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use async_trait::async_trait;
use gstreamer as gst;
use gstreamer::prelude::*;
use tokio::sync::RwLock;
use zbus::Connection;
use zbus::proxy;

use crate::config::BatteryModuleConfig;
use super::{Module, ModuleContext, ModuleItem, Urgency};

/// Global flag to track if GStreamer has been initialized
static GST_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// UPower device states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatteryState {
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
}

impl BatteryState {
    fn from_u32(value: u32) -> Self {
        match value {
            1 => BatteryState::Charging,
            2 => BatteryState::Discharging,
            3 => BatteryState::Empty,
            4 => BatteryState::FullyCharged,
            5 => BatteryState::PendingCharge,
            6 => BatteryState::PendingDischarge,
            _ => BatteryState::Unknown,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            BatteryState::Unknown => "Unknown",
            BatteryState::Charging => "Charging",
            BatteryState::Discharging => "Discharging",
            BatteryState::Empty => "Empty",
            BatteryState::FullyCharged => "Fully charged",
            BatteryState::PendingCharge => "Pending charge",
            BatteryState::PendingDischarge => "Pending discharge",
        }
    }

    fn icon_name(&self, percentage: u8) -> &'static str {
        match self {
            BatteryState::Charging => {
                if percentage >= 90 {
                    "battery-full-charging"
                } else if percentage >= 60 {
                    "battery-good-charging"
                } else if percentage >= 30 {
                    "battery-low-charging"
                } else {
                    "battery-caution-charging"
                }
            }
            BatteryState::FullyCharged => "battery-full-charged",
            _ => {
                if percentage >= 90 {
                    "battery-full"
                } else if percentage >= 60 {
                    "battery-good"
                } else if percentage >= 30 {
                    "battery-low"
                } else if percentage >= 10 {
                    "battery-caution"
                } else {
                    "battery-empty"
                }
            }
        }
    }
}

/// Proxy for the UPower DisplayDevice
#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait UPowerDevice {
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;

    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;

    #[zbus(property, name = "Type")]
    fn device_type(&self) -> zbus::Result<u32>;
}

/// Initialize GStreamer if not already done
fn ensure_gst_init() {
    if !GST_INITIALIZED.swap(true, Ordering::SeqCst) {
        if let Err(e) = gst::init() {
            tracing::error!("Failed to initialize GStreamer: {}", e);
            GST_INITIALIZED.store(false, Ordering::SeqCst);
        }
    }
}

/// Validate and expand a sound file path
/// Returns None if the path is invalid or not a regular file
fn validate_sound_path(path: &str) -> Option<String> {
    // Expand ~ to home directory
    let expanded = if path.starts_with("~/") {
        let home = dirs::home_dir()?;
        home.join(&path[2..])
    } else {
        Path::new(path).to_path_buf()
    };

    // Canonicalize to resolve any .. or symlinks (also verifies file exists)
    let canonical = match expanded.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Cannot resolve sound path '{}': {}", path, e);
            return None;
        }
    };

    // Ensure it's a regular file, not a directory or special file
    if !canonical.is_file() {
        tracing::warn!("Sound path '{}' is not a regular file", path);
        return None;
    }

    Some(canonical.to_string_lossy().to_string())
}

/// Play a sound file using GStreamer (fire and forget)
fn play_sound(path: &str) {
    ensure_gst_init();

    // Validate and expand the path (also handles path traversal prevention)
    let expanded_path = match validate_sound_path(path) {
        Some(p) => p,
        None => return,
    };

    // Create playbin element - GStreamer will handle file not found errors
    let uri = format!("file://{}", expanded_path);
    let playbin = match gst::ElementFactory::make("playbin")
        .property("uri", &uri)
        .build()
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to create playbin: {}", e);
            return;
        }
    };

    // Start playback
    if let Err(e) = playbin.set_state(gst::State::Playing) {
        tracing::error!("Failed to start sound playback: {}", e);
        return;
    }

    // Spawn a task to wait for playback to finish and clean up
    let playbin_weak = playbin.downgrade();
    std::thread::spawn(move || {
        let Some(playbin) = playbin_weak.upgrade() else {
            return;
        };

        // Get the bus and wait for EOS or error
        let bus = match playbin.bus() {
            Some(b) => b,
            None => {
                let _ = playbin.set_state(gst::State::Null);
                return;
            }
        };

        for msg in bus.iter_timed(gst::ClockTime::from_seconds(30)) {
            match msg.view() {
                gst::MessageView::Eos(_) => break,
                gst::MessageView::Error(err) => {
                    tracing::error!(
                        "Sound playback error: {} ({:?})",
                        err.error(),
                        err.debug()
                    );
                    break;
                }
                _ => {}
            }
        }

        // Clean up
        let _ = playbin.set_state(gst::State::Null);
    });
}

/// Battery module that displays battery status
pub struct BatteryModule {
    config: RwLock<BatteryModuleConfig>,
    connection: RwLock<Option<Connection>>,
    last_low_notification: RwLock<bool>,
    last_critical_notification: RwLock<bool>,
    last_full_notification: RwLock<bool>,
}

impl BatteryModule {
    pub fn new(config: BatteryModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            connection: RwLock::new(None),
            last_low_notification: RwLock::new(false),
            last_critical_notification: RwLock::new(false),
            last_full_notification: RwLock::new(false),
        }
    }

    async fn get_battery_info(&self) -> Option<(u8, BatteryState, i64)> {
        let conn_lock = self.connection.read().await;
        let connection = conn_lock.as_ref()?;

        let proxy = match UPowerDeviceProxy::new(connection).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Failed to create UPower proxy: {}", e);
                return None;
            }
        };

        // Check if battery is present
        let is_present = proxy.is_present().await.unwrap_or(false);
        if !is_present {
            tracing::debug!("No battery present");
            return None;
        }

        // Check device type (2 = Battery)
        let device_type = proxy.device_type().await.unwrap_or(0);
        if device_type != 2 {
            tracing::debug!("DisplayDevice is not a battery (type={})", device_type);
            return None;
        }

        let percentage = proxy.percentage().await.unwrap_or(0.0) as u8;
        let state = BatteryState::from_u32(proxy.state().await.unwrap_or(0));

        // Get time remaining based on state
        let time_remaining = match state {
            BatteryState::Charging => proxy.time_to_full().await.unwrap_or(0),
            BatteryState::Discharging => proxy.time_to_empty().await.unwrap_or(0),
            _ => 0,
        };

        Some((percentage, state, time_remaining))
    }

    fn format_time(seconds: i64) -> String {
        if seconds <= 0 {
            return String::new();
        }

        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;

        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m", minutes)
        }
    }

    async fn create_module_item(&self, percentage: u8, state: BatteryState, time_remaining: i64) -> ModuleItem {
        let label = format!("{}%", percentage);

        let mut tooltip_parts = vec![
            format!("Battery: {}%", percentage),
            format!("Status: {}", state.as_str()),
        ];

        let time_str = Self::format_time(time_remaining);
        if !time_str.is_empty() {
            match state {
                BatteryState::Charging => tooltip_parts.push(format!("Time to full: {}", time_str)),
                BatteryState::Discharging => tooltip_parts.push(format!("Time remaining: {}", time_str)),
                _ => {}
            }
        }

        let tooltip = tooltip_parts.join("\n");
        let icon_name = state.icon_name(percentage);

        ModuleItem {
            id: "battery:status".to_string(),
            module: "battery".to_string(),
            label,
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: Vec::new(), // Battery has no actions
        }
    }

    async fn check_and_send_notifications(&self, ctx: &ModuleContext, percentage: u8, state: BatteryState) {
        let config = self.config.read().await;

        // Check for fully charged notification
        if state == BatteryState::FullyCharged && config.notify_full_charge {
            let already_notified = *self.last_full_notification.read().await;
            if !already_notified {
                ctx.send_notification(
                    "Battery Fully Charged",
                    "Battery is fully charged. You can unplug the charger.",
                    Urgency::Low,
                );
                // Play sound if configured
                if let Some(ref sound_path) = config.full_sound {
                    play_sound(sound_path);
                }
                *self.last_full_notification.write().await = true;
            }
        } else if state != BatteryState::FullyCharged {
            // Reset full notification flag when no longer fully charged
            *self.last_full_notification.write().await = false;
        }

        // Only send low/critical notifications when discharging
        if state != BatteryState::Discharging {
            // Reset low/critical notification flags when not discharging
            *self.last_low_notification.write().await = false;
            *self.last_critical_notification.write().await = false;
            return;
        }

        // Critical battery notification
        if percentage <= config.critical_threshold {
            let already_notified = *self.last_critical_notification.read().await;
            if !already_notified {
                ctx.send_notification(
                    "Critical Battery",
                    &format!("Battery is at {}%. Connect charger immediately.", percentage),
                    Urgency::Critical,
                );
                // Play sound if configured
                if let Some(ref sound_path) = config.critical_sound {
                    play_sound(sound_path);
                }
                *self.last_critical_notification.write().await = true;
            }
        }
        // Low battery notification
        else if percentage <= config.low_threshold {
            let already_notified = *self.last_low_notification.read().await;
            if !already_notified {
                ctx.send_notification(
                    "Low Battery",
                    &format!("Battery is at {}%. Consider connecting charger.", percentage),
                    Urgency::Normal,
                );
                // Play sound if configured
                if let Some(ref sound_path) = config.low_sound {
                    play_sound(sound_path);
                }
                *self.last_low_notification.write().await = true;
            }
        }
    }
}

#[async_trait]
impl Module for BatteryModule {
    fn name(&self) -> &str {
        "battery"
    }

    fn enabled(&self) -> bool {
        // Use try_read to avoid blocking, default to true if lock is held
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Connect to system bus (UPower is on system bus)
        let connection = match Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to connect to system D-Bus for battery module: {}", e);
                return;
            }
        };

        *self.connection.write().await = Some(connection);

        // Send initial state
        if let Some((percentage, state, time_remaining)) = self.get_battery_info().await {
            let item = self.create_module_item(percentage, state, time_remaining).await;
            ctx.send_items("battery", vec![item]);
            self.check_and_send_notifications(&ctx, percentage, state).await;
        } else {
            // No battery - send empty items
            ctx.send_items("battery", vec![]);
            tracing::info!("No battery detected");
            return;
        }

        // Poll for updates (UPower PropertiesChanged signals could be used instead,
        // but polling is simpler and battery changes are infrequent)
        let poll_interval = Duration::from_secs(30);

        loop {
            tokio::time::sleep(poll_interval).await;

            if let Some((percentage, state, time_remaining)) = self.get_battery_info().await {
                let item = self.create_module_item(percentage, state, time_remaining).await;
                ctx.send_items("battery", vec![item]);
                self.check_and_send_notifications(&ctx, percentage, state).await;
            }
        }
    }

    async fn stop(&self) {
        *self.connection.write().await = None;
        tracing::info!("Battery module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // Battery module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref battery_config) = config.modules.battery {
            let mut current = self.config.write().await;
            *current = battery_config.clone();
            tracing::debug!("Battery module config reloaded");
            true
        } else {
            false
        }
    }
}
