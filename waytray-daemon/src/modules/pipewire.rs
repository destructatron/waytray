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

/// Current audio output (sink) state
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

/// Current microphone (source) state
#[derive(Debug, Clone, PartialEq)]
struct MicState {
    volume_percent: u32,
    muted: bool,
    source_name: String,
}

/// PipeWire/PulseAudio module for volume control
pub struct PipewireModule {
    config: RwLock<PipewireModuleConfig>,
    ctx: RwLock<Option<Arc<ModuleContext>>>,
}

impl PipewireModule {
    pub fn new(config: PipewireModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            ctx: RwLock::new(None),
        }
    }

    /// Send an immediate update to the client
    async fn send_update(&self) {
        let ctx_lock = self.ctx.read().await;
        if let Some(ctx) = ctx_lock.as_ref() {
            let config = self.config.read().await;
            let mut items = Vec::new();

            if let Some(state) = Self::get_audio_state() {
                drop(config); // Release before awaiting
                items.push(self.create_module_item(&state).await);
            } else {
                drop(config);
            }

            let config = self.config.read().await;
            if config.show_microphone {
                if let Some(mic_state) = Self::get_mic_state() {
                    drop(config);
                    items.push(self.create_mic_module_item(&mic_state).await);
                }
            }

            if !items.is_empty() {
                ctx.send_items("pipewire", items);
            }
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

    fn get_mic_icon_name(volume_percent: u32, muted: bool) -> &'static str {
        if muted || volume_percent == 0 {
            "microphone-sensitivity-muted"
        } else if volume_percent < 33 {
            "microphone-sensitivity-low"
        } else if volume_percent < 66 {
            "microphone-sensitivity-medium"
        } else {
            "microphone-sensitivity-high"
        }
    }

    /// Get current audio state using pactl
    fn get_audio_state() -> Option<AudioState> {
        // Get default sink name
        let default_sink = match Command::new("pactl")
            .args(["get-default-sink"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    tracing::debug!(
                        "pactl get-default-sink failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                output
            }
            Err(e) => {
                tracing::debug!("Failed to execute pactl: {}", e);
                return None;
            }
        };

        let sink_name = String::from_utf8_lossy(&default_sink.stdout)
            .trim()
            .to_string();

        if sink_name.is_empty() {
            return None;
        }

        // Get volume
        let volume_output = match Command::new("pactl")
            .args(["get-sink-volume", "@DEFAULT_SINK@"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    tracing::debug!(
                        "pactl get-sink-volume failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                output
            }
            Err(e) => {
                tracing::debug!("Failed to execute pactl: {}", e);
                return None;
            }
        };

        let volume_str = String::from_utf8_lossy(&volume_output.stdout);
        let volume_percent = Self::parse_volume(&volume_str).unwrap_or(0);

        // Get mute status
        let mute_output = match Command::new("pactl")
            .args(["get-sink-mute", "@DEFAULT_SINK@"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    tracing::debug!(
                        "pactl get-sink-mute failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                output
            }
            Err(e) => {
                tracing::debug!("Failed to execute pactl: {}", e);
                return None;
            }
        };

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

    /// Get current microphone (source) state using pactl
    fn get_mic_state() -> Option<MicState> {
        // Get default source name
        let default_source = match Command::new("pactl")
            .args(["get-default-source"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    tracing::debug!(
                        "pactl get-default-source failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                output
            }
            Err(e) => {
                tracing::debug!("Failed to execute pactl: {}", e);
                return None;
            }
        };

        let source_name = String::from_utf8_lossy(&default_source.stdout)
            .trim()
            .to_string();

        if source_name.is_empty() {
            return None;
        }

        // Skip monitor sources (they're not real microphones)
        if source_name.contains(".monitor") {
            tracing::debug!("Default source is a monitor, no microphone available");
            return None;
        }

        // Get volume
        let volume_output = match Command::new("pactl")
            .args(["get-source-volume", "@DEFAULT_SOURCE@"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    tracing::debug!(
                        "pactl get-source-volume failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                output
            }
            Err(e) => {
                tracing::debug!("Failed to execute pactl: {}", e);
                return None;
            }
        };

        let volume_str = String::from_utf8_lossy(&volume_output.stdout);
        let volume_percent = Self::parse_volume(&volume_str).unwrap_or(0);

        // Get mute status
        let mute_output = match Command::new("pactl")
            .args(["get-source-mute", "@DEFAULT_SOURCE@"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    tracing::debug!(
                        "pactl get-source-mute failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                output
            }
            Err(e) => {
                tracing::debug!("Failed to execute pactl: {}", e);
                return None;
            }
        };

        let mute_str = String::from_utf8_lossy(&mute_output.stdout);
        let muted = mute_str.contains("yes");

        // Get source description for a nicer name
        let source_description = Self::get_source_description(&source_name)
            .unwrap_or_else(|| source_name.clone());

        Some(MicState {
            volume_percent,
            muted,
            source_name: source_description,
        })
    }

    /// Get source description from pactl list-sources
    fn get_source_description(source_name: &str) -> Option<String> {
        let output = Command::new("pactl")
            .args(["list", "sources"])
            .output()
            .ok()?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut in_target_source = false;

        for line in output_str.lines() {
            let trimmed = line.trim();

            // Check if this is our source
            if trimmed.starts_with("Name:") {
                let name = trimmed.strip_prefix("Name:")?.trim();
                in_target_source = name == source_name;
            }

            // Get description if we're in the right source
            if in_target_source && trimmed.starts_with("Description:") {
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

    async fn create_mic_module_item(&self, state: &MicState) -> ModuleItem {
        let config = self.config.read().await;

        let label = if config.show_mic_volume {
            if state.muted {
                "Muted".to_string()
            } else {
                format!("{}%", state.volume_percent)
            }
        } else {
            String::new()
        };

        let icon_name = Self::get_mic_icon_name(state.volume_percent, state.muted);

        let tooltip = if state.muted {
            format!("Microphone: {}% (Muted)\nInput: {}", state.volume_percent, state.source_name)
        } else {
            format!("Microphone: {}%\nInput: {}", state.volume_percent, state.source_name)
        };

        ModuleItem {
            id: "pipewire:microphone".to_string(),
            module: "pipewire".to_string(),
            label,
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: vec![
                ItemAction::default_action("mic_toggle_mute", if state.muted { "Unmute Microphone" } else { "Mute Microphone" }),
                ItemAction::new("mic_volume_up", "Microphone Volume Up"),
                ItemAction::new("mic_volume_down", "Microphone Volume Down"),
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

        // Store context for use in invoke_action
        *self.ctx.write().await = Some(ctx.clone());

        // Get initial states
        let mut last_audio_state = Self::get_audio_state().unwrap_or_default();
        let mut last_mic_state: Option<MicState> = None;

        // Send initial state
        let mut items = vec![self.create_module_item(&last_audio_state).await];

        let config = self.config.read().await;
        if config.show_microphone {
            if let Some(mic_state) = Self::get_mic_state() {
                last_mic_state = Some(mic_state.clone());
                drop(config);
                items.push(self.create_mic_module_item(&mic_state).await);
            }
        }
        ctx.send_items("pipewire", items);

        // Poll for changes
        let poll_interval = Duration::from_millis(500);

        loop {
            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(poll_interval) => {
                    let current_audio_state = Self::get_audio_state();
                    let config = self.config.read().await;
                    let current_mic_state = if config.show_microphone {
                        Self::get_mic_state()
                    } else {
                        None
                    };
                    drop(config);

                    // Check if either state changed
                    let audio_changed = current_audio_state.as_ref() != Some(&last_audio_state);
                    let mic_changed = current_mic_state != last_mic_state;

                    if audio_changed || mic_changed {
                        let mut items = Vec::new();

                        if let Some(ref state) = current_audio_state {
                            items.push(self.create_module_item(state).await);
                            last_audio_state = state.clone();
                        }

                        if let Some(ref state) = current_mic_state {
                            items.push(self.create_mic_module_item(state).await);
                        }
                        last_mic_state = current_mic_state;

                        if !items.is_empty() {
                            ctx.send_items("pipewire", items);
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("PipeWire module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, action_id: &str, _x: i32, _y: i32) {
        let config = self.config.read().await;

        let action_performed = match action_id {
            // Output (sink) actions
            "toggle_mute" => {
                let _ = Command::new("pactl")
                    .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
                    .output();
                true
            }
            "volume_up" => {
                let step = config.scroll_step;
                let max = config.max_volume;

                // Check if we're already at max before increasing
                if let Some(state) = Self::get_audio_state() {
                    if state.volume_percent >= max {
                        return;
                    }
                }

                let _ = Command::new("pactl")
                    .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("+{}%", step)])
                    .output();
                true
            }
            "volume_down" => {
                let step = config.scroll_step;
                let _ = Command::new("pactl")
                    .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("-{}%", step)])
                    .output();
                true
            }
            // Microphone (source) actions
            "mic_toggle_mute" => {
                let _ = Command::new("pactl")
                    .args(["set-source-mute", "@DEFAULT_SOURCE@", "toggle"])
                    .output();
                true
            }
            "mic_volume_up" => {
                let step = config.mic_scroll_step;
                let max = config.mic_max_volume;

                // Check if we're already at max before increasing
                if let Some(state) = Self::get_mic_state() {
                    if state.volume_percent >= max {
                        return;
                    }
                }

                let _ = Command::new("pactl")
                    .args(["set-source-volume", "@DEFAULT_SOURCE@", &format!("+{}%", step)])
                    .output();
                true
            }
            "mic_volume_down" => {
                let step = config.mic_scroll_step;
                let _ = Command::new("pactl")
                    .args(["set-source-volume", "@DEFAULT_SOURCE@", &format!("-{}%", step)])
                    .output();
                true
            }
            _ => {
                tracing::warn!("Unknown action: {}", action_id);
                false
            }
        };

        // Immediately refresh the display after an action
        if action_performed {
            drop(config); // Release the lock before send_update
            self.send_update().await;
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
