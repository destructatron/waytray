//! Power Profiles module - displays and controls power profile via power-profiles-daemon
//!
//! Integrates with power-profiles-daemon via D-Bus to show and switch between
//! power-saver, balanced, and performance profiles.

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::RwLock;
use zbus::Connection;
use zbus::proxy;

use crate::config::PowerProfilesModuleConfig;
use super::{Module, ModuleContext, ModuleItem, ItemAction};

/// Power profile types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerProfile {
    PowerSaver,
    Balanced,
    Performance,
}

impl PowerProfile {
    fn from_str(s: &str) -> Self {
        match s {
            "power-saver" => PowerProfile::PowerSaver,
            "performance" => PowerProfile::Performance,
            _ => PowerProfile::Balanced,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "power-saver",
            PowerProfile::Balanced => "balanced",
            PowerProfile::Performance => "performance",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "Power Saver",
            PowerProfile::Balanced => "Balanced",
            PowerProfile::Performance => "Performance",
        }
    }

    fn icon_name(&self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "power-profile-power-saver-symbolic",
            PowerProfile::Balanced => "power-profile-balanced-symbolic",
            PowerProfile::Performance => "power-profile-performance-symbolic",
        }
    }

    /// Get the next profile in the cycle order
    fn next(&self) -> Self {
        match self {
            PowerProfile::PowerSaver => PowerProfile::Balanced,
            PowerProfile::Balanced => PowerProfile::Performance,
            PowerProfile::Performance => PowerProfile::PowerSaver,
        }
    }
}

/// D-Bus proxy for power-profiles-daemon
#[proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
trait PowerProfilesDaemon {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn performance_degraded(&self) -> zbus::Result<String>;
}

/// Current power profiles state
#[derive(Debug, Clone, PartialEq)]
struct PowerProfilesState {
    profile: PowerProfile,
    degraded_reason: String,
}

impl Default for PowerProfilesState {
    fn default() -> Self {
        Self {
            profile: PowerProfile::Balanced,
            degraded_reason: String::new(),
        }
    }
}

/// Power Profiles module
pub struct PowerProfilesModule {
    config: RwLock<PowerProfilesModuleConfig>,
    connection: RwLock<Option<Connection>>,
}

impl PowerProfilesModule {
    pub fn new(config: PowerProfilesModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            connection: RwLock::new(None),
        }
    }

    async fn get_state(&self) -> Option<PowerProfilesState> {
        let conn_lock = self.connection.read().await;
        let connection = conn_lock.as_ref()?;

        let proxy = match PowerProfilesDaemonProxy::new(connection).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Failed to create power-profiles-daemon proxy: {}", e);
                return None;
            }
        };

        let profile_str = proxy.active_profile().await.ok()?;
        let profile = PowerProfile::from_str(&profile_str);

        let degraded_reason = proxy.performance_degraded().await.unwrap_or_default();

        Some(PowerProfilesState {
            profile,
            degraded_reason,
        })
    }

    async fn set_profile(&self, profile: PowerProfile) -> bool {
        let conn_lock = self.connection.read().await;
        let Some(connection) = conn_lock.as_ref() else {
            return false;
        };

        let proxy = match PowerProfilesDaemonProxy::new(connection).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to create power-profiles-daemon proxy: {}", e);
                return false;
            }
        };

        match proxy.set_active_profile(profile.as_str()).await {
            Ok(()) => {
                tracing::info!("Set power profile to {}", profile.display_name());
                true
            }
            Err(e) => {
                tracing::error!("Failed to set power profile: {}", e);
                false
            }
        }
    }

    fn create_module_item(state: &PowerProfilesState) -> ModuleItem {
        let label = state.profile.display_name().to_string();
        let icon_name = state.profile.icon_name();

        let mut tooltip = format!("Power Profile: {}", state.profile.display_name());

        if !state.degraded_reason.is_empty() {
            let reason_display = match state.degraded_reason.as_str() {
                "lap-detected" => "Lap detected",
                "high-operating-temperature" => "High temperature",
                other => other,
            };
            tooltip.push_str(&format!("\nPerformance degraded: {}", reason_display));
        }

        tooltip.push_str("\n\nPress Enter to cycle profiles");

        ModuleItem {
            id: "power_profiles:status".to_string(),
            module: "power_profiles".to_string(),
            label,
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: vec![
                ItemAction::default_action("cycle", "Cycle Profile"),
                ItemAction::new("context_menu", "Select Profile"),
            ],
        }
    }
}

#[async_trait]
impl Module for PowerProfilesModule {
    fn name(&self) -> &str {
        "power_profiles"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Connect to system bus (power-profiles-daemon is on system bus)
        let connection = match Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to connect to system D-Bus for power profiles: {}", e);
                return;
            }
        };

        *self.connection.write().await = Some(connection);

        // Check if power-profiles-daemon is available
        let initial_state = match self.get_state().await {
            Some(state) => state,
            None => {
                tracing::info!("power-profiles-daemon not available");
                ctx.send_items("power_profiles", vec![]);
                return;
            }
        };

        // Send initial state
        let item = Self::create_module_item(&initial_state);
        ctx.send_items("power_profiles", vec![item]);

        // Poll for changes
        let poll_interval = Duration::from_secs(2);
        let mut last_state = initial_state;

        loop {
            tokio::time::sleep(poll_interval).await;

            if let Some(current_state) = self.get_state().await {
                if current_state != last_state {
                    let item = Self::create_module_item(&current_state);
                    ctx.send_items("power_profiles", vec![item]);
                    last_state = current_state;
                }
            }
        }
    }

    async fn stop(&self) {
        *self.connection.write().await = None;
        tracing::info!("Power profiles module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, action_id: &str, _x: i32, _y: i32) {
        match action_id {
            "cycle" => {
                // Cycle to next profile
                if let Some(state) = self.get_state().await {
                    let next_profile = state.profile.next();
                    self.set_profile(next_profile).await;
                }
            }
            "context_menu" => {
                // Context menu is handled via get_menu_items/activate_menu_item
            }
            _ => {
                tracing::warn!("Unknown action: {}", action_id);
            }
        }
    }

    async fn get_menu_items(&self, _item_id: &str) -> anyhow::Result<Vec<crate::dbusmenu::MenuItem>> {
        let current_state = self.get_state().await;
        let current_profile = current_state.map(|s| s.profile).unwrap_or(PowerProfile::Balanced);

        let profiles = [
            (1, PowerProfile::PowerSaver),
            (2, PowerProfile::Balanced),
            (3, PowerProfile::Performance),
        ];

        let items = profiles
            .iter()
            .map(|(id, profile)| {
                crate::dbusmenu::MenuItem {
                    id: *id,
                    label: profile.display_name().to_string(),
                    enabled: true,
                    visible: true,
                    item_type: "standard".to_string(),
                    icon_name: Some(profile.icon_name().to_string()),
                    toggle_type: Some("radio".to_string()),
                    toggle_state: if *profile == current_profile { 1 } else { 0 },
                    children: vec![],
                }
            })
            .collect();

        Ok(items)
    }

    async fn activate_menu_item(&self, _item_id: &str, menu_item_id: i32) -> anyhow::Result<()> {
        let profile = match menu_item_id {
            1 => PowerProfile::PowerSaver,
            2 => PowerProfile::Balanced,
            3 => PowerProfile::Performance,
            _ => anyhow::bail!("Unknown menu item id: {}", menu_item_id),
        };

        self.set_profile(profile).await;
        Ok(())
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref power_profiles_config) = config.modules.power_profiles {
            let mut current = self.config.write().await;
            *current = power_profiles_config.clone();
            tracing::debug!("Power profiles module config reloaded");
            true
        } else {
            false
        }
    }
}
