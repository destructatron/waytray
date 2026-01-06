//! Privacy module - shows which apps are using the microphone

use std::collections::HashSet;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::PrivacyModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

#[derive(Default)]
struct SourceOutputInfo {
    app_name: Option<String>,
    process_binary: Option<String>,
    corked: Option<bool>,
}

/// Privacy module that surfaces active microphone usage
pub struct PrivacyModule {
    config: RwLock<PrivacyModuleConfig>,
}

impl PrivacyModule {
    pub fn new(config: PrivacyModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
        }
    }

    fn parse_property_value(line: &str, key: &str) -> Option<String> {
        let prefix = format!("{key} =");
        let value = line.strip_prefix(&prefix)?.trim();
        let value = value.trim_matches('"').trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }

    fn push_app(
        info: &SourceOutputInfo,
        apps: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        if info.corked == Some(true) {
            return;
        }

        let name = info
            .app_name
            .as_ref()
            .or(info.process_binary.as_ref());

        if let Some(name) = name {
            if seen.insert(name.clone()) {
                apps.push(name.clone());
            }
        }
    }

    fn parse_source_outputs(output: &str) -> Vec<String> {
        let mut apps = Vec::new();
        let mut seen = HashSet::new();
        let mut current = SourceOutputInfo::default();

        for line in output.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("Source Output #") {
                Self::push_app(&current, &mut apps, &mut seen);
                current = SourceOutputInfo::default();
                continue;
            }

            if let Some(value) = Self::parse_property_value(trimmed, "application.name") {
                current.app_name = Some(value);
                continue;
            }

            if let Some(value) =
                Self::parse_property_value(trimmed, "application.process.binary")
            {
                current.process_binary = Some(value);
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("Corked:") {
                let corked = value.trim().eq_ignore_ascii_case("yes");
                current.corked = Some(corked);
            }
        }

        Self::push_app(&current, &mut apps, &mut seen);
        apps
    }

    fn get_active_apps() -> Option<Vec<String>> {
        let output = Command::new("pactl")
            .args(["list", "source-outputs"])
            .output()
            .ok()?;

        if !output.status.success() {
            tracing::debug!(
                "pactl list source-outputs failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        Some(Self::parse_source_outputs(&output_str))
    }

    fn format_label(apps: &[String]) -> String {
        if apps.len() == 1 {
            apps[0].clone()
        } else if apps.is_empty() {
            "Mic idle".to_string()
        } else {
            format!("{} +{}", apps[0], apps.len() - 1)
        }
    }

    fn format_tooltip(apps: &[String]) -> String {
        if apps.is_empty() {
            "No apps are using the microphone.".to_string()
        } else {
            let mut tooltip = String::from("Microphone in use by:");
            for app in apps {
                tooltip.push('\n');
                tooltip.push_str(app);
            }
            tooltip
        }
    }

    async fn build_items(&self, apps: Vec<String>) -> Vec<ModuleItem> {
        let config = self.config.read().await;

        if apps.is_empty() && !config.show_when_idle {
            return Vec::new();
        }

        let icon_name = if apps.is_empty() {
            "microphone-sensitivity-muted"
        } else {
            "microphone-sensitivity-high"
        };

        vec![ModuleItem {
            id: "privacy:microphone".to_string(),
            module: "privacy".to_string(),
            label: Self::format_label(&apps),
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(Self::format_tooltip(&apps)),
            actions: Vec::new(),
        }]
    }
}

#[async_trait]
impl Module for PrivacyModule {
    fn name(&self) -> &str {
        "privacy"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        if Command::new("pactl").arg("--version").output().is_err() {
            tracing::error!("pactl not found. Install pulseaudio-utils or pipewire-pulse.");
            return;
        }

        let mut last_apps: Vec<String> = Vec::new();

        if let Some(apps) = Self::get_active_apps() {
            let items = self.build_items(apps.clone()).await;
            ctx.send_items("privacy", items);
            last_apps = apps;
        }

        loop {
            let interval = {
                let config = self.config.read().await;
                Duration::from_secs(config.interval_seconds)
            };

            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(interval) => {
                    if let Some(apps) = Self::get_active_apps() {
                        if apps != last_apps {
                            let items = self.build_items(apps.clone()).await;
                            ctx.send_items("privacy", items);
                            last_apps = apps;
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("Privacy module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // Privacy module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref privacy_config) = config.modules.privacy {
            let mut current = self.config.write().await;
            *current = privacy_config.clone();
            tracing::debug!("Privacy module config reloaded");
            true
        } else {
            false
        }
    }
}
