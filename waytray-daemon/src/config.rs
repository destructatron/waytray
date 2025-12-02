use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub modules: ModulesConfig,
    pub notifications: NotificationsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            modules: ModulesConfig::default(),
            notifications: NotificationsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModulesConfig {
    /// Module ordering - modules appear in this order in the panel
    /// Modules not listed here appear after listed ones
    #[serde(default)]
    pub order: Vec<String>,
    pub tray: TrayModuleConfig,
    pub battery: Option<BatteryModuleConfig>,
    pub clock: Option<ClockModuleConfig>,
    pub system: Option<SystemModuleConfig>,
    pub weather: Option<WeatherModuleConfig>,
    #[serde(default)]
    pub scripts: Vec<ScriptModuleConfig>,
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            order: vec!["tray".to_string()],
            tray: TrayModuleConfig::default(),
            battery: None,
            clock: None,
            system: None,
            weather: None,
            scripts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TrayModuleConfig {
    pub enabled: bool,
}

impl Default for TrayModuleConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BatteryModuleConfig {
    pub enabled: bool,
    /// Battery percentage threshold for low battery notification
    pub low_threshold: u8,
    /// Battery percentage threshold for critical battery notification
    pub critical_threshold: u8,
    /// Whether to notify when battery is fully charged
    pub notify_full_charge: bool,
}

impl Default for BatteryModuleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            low_threshold: 20,
            critical_threshold: 10,
            notify_full_charge: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClockModuleConfig {
    pub enabled: bool,
    /// Time format string (strftime format)
    /// Default: "%H:%M" (24-hour time)
    /// Examples: "%I:%M %p" (12-hour with AM/PM), "%H:%M:%S" (with seconds)
    pub format: String,
    /// Date format for tooltip (strftime format)
    /// Default: "%A, %B %d, %Y" (e.g., "Monday, January 15, 2024")
    pub date_format: String,
}

impl Default for ClockModuleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format: "%H:%M".to_string(),
            date_format: "%A, %B %d, %Y".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SystemModuleConfig {
    pub enabled: bool,
    pub show_cpu: bool,
    pub show_memory: bool,
    /// Update interval in seconds
    pub interval_seconds: u64,
}

impl Default for SystemModuleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_cpu: true,
            show_memory: true,
            interval_seconds: 5,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WeatherModuleConfig {
    pub enabled: bool,
    /// OpenWeatherMap API key
    pub api_key: String,
    /// Location for weather (city name or coordinates)
    pub location: String,
    /// Update interval in seconds
    pub interval_seconds: u64,
    /// Temperature unit: "celsius" or "fahrenheit"
    pub units: String,
}

impl Default for WeatherModuleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            location: String::new(),
            interval_seconds: 900, // 15 minutes
            units: "celsius".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptModuleConfig {
    /// Unique name for this script module
    pub name: String,
    /// Command to execute (shell command)
    pub command: String,
    /// Update interval in seconds
    pub interval_seconds: u64,
    /// Icon name from theme (optional)
    pub icon: Option<String>,
    /// Static tooltip text (optional)
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    pub enabled: bool,
    /// Notification timeout in milliseconds (0 = no timeout)
    pub timeout_ms: u32,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 5000,
        }
    }
}

impl Config {
    /// Load configuration from the default path (~/.config/waytray/config.toml)
    /// Returns default config if file doesn't exist
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        Self::load_from_path(&path)
    }

    /// Load configuration from a specific path
    pub fn load_from_path(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            tracing::info!("Config file not found at {:?}, using defaults", path);
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;

        tracing::info!("Loaded config from {:?}", path);
        Ok(config)
    }

    /// Get the default config file path
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("waytray")
            .join("config.toml")
    }

    /// Get the list of module names in order
    pub fn module_order(&self) -> Vec<String> {
        let mut order = self.modules.order.clone();

        // Add any enabled modules not in the order list
        if self.modules.tray.enabled && !order.contains(&"tray".to_string()) {
            order.push("tray".to_string());
        }
        if let Some(ref battery) = self.modules.battery {
            if battery.enabled && !order.contains(&"battery".to_string()) {
                order.push("battery".to_string());
            }
        }
        if let Some(ref clock) = self.modules.clock {
            if clock.enabled && !order.contains(&"clock".to_string()) {
                order.push("clock".to_string());
            }
        }
        if let Some(ref system) = self.modules.system {
            if system.enabled && !order.contains(&"system".to_string()) {
                order.push("system".to_string());
            }
        }
        if let Some(ref weather) = self.modules.weather {
            if weather.enabled && !order.contains(&"weather".to_string()) {
                order.push("weather".to_string());
            }
        }
        for script in &self.modules.scripts {
            let script_name = format!("script:{}", script.name);
            if !order.contains(&script_name) {
                order.push(script_name);
            }
        }

        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.modules.tray.enabled);
        assert!(config.modules.battery.is_none());
        assert!(config.notifications.enabled);
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[modules.tray]
enabled = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.modules.tray.enabled);
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[modules]
order = ["tray", "battery", "system"]

[modules.tray]
enabled = true

[modules.battery]
enabled = true
low_threshold = 15
critical_threshold = 5

[modules.system]
enabled = true
show_cpu = true
show_memory = true
interval_seconds = 10

[[modules.scripts]]
name = "my-script"
command = "/path/to/script.sh"
interval_seconds = 30
icon = "utilities-terminal"

[notifications]
enabled = true
timeout_ms = 3000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.modules.tray.enabled);
        assert!(config.modules.battery.as_ref().unwrap().enabled);
        assert_eq!(config.modules.battery.as_ref().unwrap().low_threshold, 15);
        assert!(config.modules.system.as_ref().unwrap().show_cpu);
        assert_eq!(config.modules.scripts.len(), 1);
        assert_eq!(config.modules.scripts[0].name, "my-script");
        assert_eq!(config.notifications.timeout_ms, 3000);
    }

    #[test]
    fn test_module_order() {
        let toml = r#"
[modules]
order = ["battery", "tray"]

[modules.tray]
enabled = true

[modules.battery]
enabled = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let order = config.module_order();
        assert_eq!(order[0], "battery");
        assert_eq!(order[1], "tray");
    }
}
