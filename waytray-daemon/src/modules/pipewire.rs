//! PipeWire/PulseAudio module - displays volume status and allows control
//!
//! Uses pactl commands to communicate with PulseAudio or PipeWire (via pipewire-pulse).
//! This approach is more reliable than direct libpulse bindings for our use case.

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::PipewireModuleConfig;
use super::{Module, ModuleContext, ModuleItem, ItemAction};

/// Current audio state
#[derive(Debug, Clone, PartialEq)]
struct AudioState {
    volume_percent: u32,
    muted: bool,
    sink_name: String,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            volume_percent: 0,
            muted: false,
            sink_name: String::new(),
        }
    }
}

/// PipeWire/PulseAudio module for volume control
pub struct PipewireModule {
    config: RwLock<PipewireModuleConfig>,
}

impl PipewireModule {
    pub fn new(config: PipewireModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
        }
    }

    fn get_icon_name(volume_percent: u32, muted: bool) -> &'static str {
        if muted {
            "audio-volume-muted"
        } else if volume_percent == 0 {
            "audio-volume-muted"
        } else if volume_percent < 33 {
            "audio-volume-low"
        } else if volume_percent < 66 {
            "audio-volume-medium"
        } else {
            "audio-volume-high"
        }
    }

    /// Get current audio state using pactl
    fn get_audio_state() -> Option<AudioState> {
        // Get default sink name
        let default_sink = Command::new("pactl")
            .args(["get-default-sink"])
            .output()
            .ok()?;

        let sink_name = String::from_utf8_lossy(&default_sink.stdout)
            .trim()
            .to_string();

        if sink_name.is_empty() {
            return None;
        }

        // Get volume
        let volume_output = Command::new("pactl")
            .args(["get-sink-volume", "@DEFAULT_SINK@"])
            .output()
            .ok()?;

        let volume_str = String::from_utf8_lossy(&volume_output.stdout);
        let volume_percent = Self::parse_volume(&volume_str).unwrap_or(0);

        // Get mute status
        let mute_output = Command::new("pactl")
            .args(["get-sink-mute", "@DEFAULT_SINK@"])
            .output()
            .ok()?;

        let mute_str = String::from_utf8_lossy(&mute_output.stdout);
        let muted = mute_str.contains("yes");

        // Get sink description for a nicer name
        let sink_description = Self::get_sink_description(&sink_name)
            .unwrap_or_else(|| sink_name.clone());

        Some(AudioState {
            volume_percent,
            muted,
            sink_name: sink_description,
        })
    }

    /// Parse volume percentage from pactl output
    /// Format: "Volume: front-left: 65536 / 100% / -0.00 dB,   front-right: 65536 / 100% / -0.00 dB"
    fn parse_volume(output: &str) -> Option<u32> {
        // Find the first percentage value
        for part in output.split('/') {
            let trimmed = part.trim();
            if let Some(percent_str) = trimmed.strip_suffix('%') {
                if let Ok(percent) = percent_str.trim().parse::<u32>() {
                    return Some(percent);
                }
            }
        }
        None
    }

    /// Get sink description from pactl list-sinks
    fn get_sink_description(sink_name: &str) -> Option<String> {
        let output = Command::new("pactl")
            .args(["list", "sinks"])
            .output()
            .ok()?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut in_target_sink = false;

        for line in output_str.lines() {
            let trimmed = line.trim();

            // Check if this is our sink
            if trimmed.starts_with("Name:") {
                let name = trimmed.strip_prefix("Name:")?.trim();
                in_target_sink = name == sink_name;
            }

            // Get description if we're in the right sink
            if in_target_sink && trimmed.starts_with("Description:") {
                return Some(trimmed.strip_prefix("Description:")?.trim().to_string());
            }
        }

        None
    }

    async fn create_module_item(&self, state: &AudioState) -> ModuleItem {
        let config = self.config.read().await;

        let label = if config.show_volume {
            if state.muted {
                "Muted".to_string()
            } else {
                format!("{}%", state.volume_percent)
            }
        } else {
            String::new()
        };

        let icon_name = Self::get_icon_name(state.volume_percent, state.muted);

        let tooltip = if state.muted {
            format!("Volume: {}% (Muted)\nOutput: {}", state.volume_percent, state.sink_name)
        } else {
            format!("Volume: {}%\nOutput: {}", state.volume_percent, state.sink_name)
        };

        ModuleItem {
            id: "pipewire:volume".to_string(),
            module: "pipewire".to_string(),
            label,
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: vec![
                ItemAction::default_action("toggle_mute", if state.muted { "Unmute" } else { "Mute" }),
                ItemAction::new("volume_up", "Volume Up"),
                ItemAction::new("volume_down", "Volume Down"),
            ],
        }
    }
}

#[async_trait]
impl Module for PipewireModule {
    fn name(&self) -> &str {
        "pipewire"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Check if pactl is available
        if Command::new("pactl").arg("--version").output().is_err() {
            tracing::error!("pactl not found. Install pulseaudio-utils or pipewire-pulse.");
            return;
        }

        // Get initial state
        let mut last_state = Self::get_audio_state().unwrap_or_default();

        // Send initial state
        let item = self.create_module_item(&last_state).await;
        ctx.send_items("pipewire", vec![item]);

        // Poll for changes
        let poll_interval = Duration::from_millis(500);

        loop {
            tokio::time::sleep(poll_interval).await;

            if let Some(current_state) = Self::get_audio_state() {
                // Only update if state changed
                if current_state != last_state {
                    let item = self.create_module_item(&current_state).await;
                    ctx.send_items("pipewire", vec![item]);
                    last_state = current_state;
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("PipeWire module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, action_id: &str, _x: i32, _y: i32) {
        let config = self.config.read().await;

        match action_id {
            "toggle_mute" => {
                let _ = Command::new("pactl")
                    .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
                    .spawn();
            }
            "volume_up" => {
                let step = config.scroll_step;
                let max = config.max_volume;

                // Use relative adjustment to preserve channel balance
                // Check if we're already at max before increasing
                if let Some(state) = Self::get_audio_state() {
                    if state.volume_percent >= max {
                        // Already at or above max, don't increase
                        return;
                    }
                }

                let _ = Command::new("pactl")
                    .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("+{}%", step)])
                    .spawn();
            }
            "volume_down" => {
                let step = config.scroll_step;
                let _ = Command::new("pactl")
                    .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("-{}%", step)])
                    .spawn();
            }
            _ => {
                tracing::warn!("Unknown action: {}", action_id);
            }
        }
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref pipewire_config) = config.modules.pipewire {
            let mut current = self.config.write().await;
            *current = pipewire_config.clone();
            tracing::debug!("PipeWire module config reloaded");
            true
        } else {
            false
        }
    }
}
