//! Scripts module - run custom scripts and display their output

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

use crate::config::{ScriptMode, ScriptModuleConfig};
use super::{ItemAction, Module, ModuleContext, ModuleItem};

/// Parsed script output
#[derive(Debug, Clone, Default, Deserialize)]
struct ScriptOutput {
    label: String,
    #[serde(default)]
    tooltip: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    actions: Vec<ScriptAction>,
}

/// Action defined in script JSON output
#[derive(Debug, Clone, Deserialize)]
struct ScriptAction {
    id: String,
    command: String,
}

/// State for a single script
struct ScriptState {
    config: ScriptModuleConfig,
    /// Last parsed output from the script
    last_output: Option<ScriptOutput>,
    /// Watch mode child process handle
    watch_child: Option<Child>,
}

/// Scripts module that executes custom scripts and displays their output
pub struct ScriptsModule {
    /// All script configurations and their state
    scripts: Arc<RwLock<HashMap<String, ScriptState>>>,
    /// Action commands keyed by "script_id:action_id"
    action_commands: Arc<RwLock<HashMap<String, String>>>,
}

impl ScriptsModule {
    pub fn new(configs: Vec<ScriptModuleConfig>) -> Self {
        let mut scripts = HashMap::new();

        for config in configs {
            if !config.enabled {
                tracing::debug!("Script '{}' is disabled, skipping", config.id);
                continue;
            }

            // Validate script path exists
            if !Path::new(&config.path).exists() {
                tracing::warn!(
                    "Script '{}' path does not exist: {}",
                    config.id,
                    config.path
                );
                continue;
            }

            tracing::info!("Registered script '{}' with mode {:?}", config.id, config.mode);
            scripts.insert(
                config.id.clone(),
                ScriptState {
                    config,
                    last_output: None,
                    watch_child: None,
                },
            );
        }

        Self {
            scripts: Arc::new(RwLock::new(scripts)),
            action_commands: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Run a script once and capture its output
    async fn run_script(&self, path: &str) -> Option<String> {
        let output = Command::new(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!("Script {} failed: {}", path, stderr.trim());
                    return None;
                }
                Some(String::from_utf8_lossy(&output.stdout).to_string())
            }
            Err(e) => {
                tracing::warn!("Failed to execute script {}: {}", path, e);
                None
            }
        }
    }

    /// Parse script output (auto-detect JSON or line format)
    fn parse_output(output: &str, default_icon: Option<&str>) -> ScriptOutput {
        let trimmed = output.trim();

        // Try JSON first if output looks like JSON
        if trimmed.starts_with('{') {
            if let Ok(mut parsed) = serde_json::from_str::<ScriptOutput>(trimmed) {
                // Use default icon if not specified in output
                if parsed.icon.is_none() {
                    parsed.icon = default_icon.map(String::from);
                }
                return parsed;
            }
            tracing::debug!("Failed to parse JSON output, falling back to line format");
        }

        // Line-based format: first line = label, second line = tooltip
        let mut lines = trimmed.lines();
        let label = lines.next().unwrap_or("").to_string();
        let tooltip = lines.next().map(|s| s.to_string());

        ScriptOutput {
            label,
            tooltip,
            icon: default_icon.map(String::from),
            actions: Vec::new(),
        }
    }

    /// Create a ModuleItem from script output
    fn create_item(script_id: &str, output: &ScriptOutput) -> ModuleItem {
        let mut item = ModuleItem::new("scripts", script_id, &output.label);

        if let Some(ref icon) = output.icon {
            item = item.with_icon_name(icon);
        }

        if let Some(ref tooltip) = output.tooltip {
            item = item.with_tooltip(tooltip);
        }

        // Add actions
        for action in &output.actions {
            let item_action = if action.id == "Activate" {
                ItemAction::default_action(&action.id, &action.id)
            } else {
                ItemAction::new(&action.id, &action.id)
            };
            item = item.with_action(item_action);
        }

        item
    }

    /// Store action commands for later execution
    async fn store_actions(&self, script_id: &str, output: &ScriptOutput) {
        let mut commands = self.action_commands.write().await;
        for action in &output.actions {
            let key = format!("{}:{}", script_id, action.id);
            commands.insert(key, action.command.clone());
        }
    }

    /// Update a single script and return its item
    async fn update_script(&self, script_id: &str) -> Option<ModuleItem> {
        let scripts = self.scripts.read().await;
        let state = scripts.get(script_id)?;

        let raw_output = self.run_script(&state.config.path).await?;
        let output = Self::parse_output(&raw_output, state.config.icon.as_deref());

        // Store actions
        self.store_actions(script_id, &output).await;

        // Store last output
        drop(scripts);
        let mut scripts = self.scripts.write().await;
        if let Some(state) = scripts.get_mut(script_id) {
            state.last_output = Some(output.clone());
        }

        Some(Self::create_item(script_id, &output))
    }

    /// Get all current items from cached outputs
    async fn get_all_items(&self) -> Vec<ModuleItem> {
        let scripts = self.scripts.read().await;
        let mut items = Vec::new();

        for (script_id, state) in scripts.iter() {
            if let Some(ref output) = state.last_output {
                items.push(Self::create_item(script_id, output));
            }
        }

        items
    }

    /// Start watch mode for a script (spawns long-running process)
    async fn start_watch_script(
        &self,
        script_id: String,
        path: String,
        default_icon: Option<String>,
        ctx: Arc<ModuleContext>,
    ) {
        let spawn_result = Command::new(&path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to spawn watch script {}: {}", path, e);
                return;
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                tracing::error!("Failed to get stdout for watch script {}", path);
                return;
            }
        };

        // Store the child process handle
        {
            let mut scripts = self.scripts.write().await;
            if let Some(state) = scripts.get_mut(&script_id) {
                state.watch_child = Some(child);
            }
        }

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let cancellation = ctx.cancellation_token();
        let action_commands = self.action_commands.clone();
        let scripts = self.scripts.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => {
                        tracing::debug!("Watch script {} cancelled", script_id);
                        break;
                    }
                    result = lines.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                let output = Self::parse_output(&line, default_icon.as_deref());

                                // Store actions
                                {
                                    let mut commands = action_commands.write().await;
                                    for action in &output.actions {
                                        let key = format!("{}:{}", script_id, action.id);
                                        commands.insert(key, action.command.clone());
                                    }
                                }

                                // Store last output and get all items
                                let all_items = {
                                    let mut scripts_lock = scripts.write().await;
                                    if let Some(state) = scripts_lock.get_mut(&script_id) {
                                        state.last_output = Some(output);
                                    }
                                    // Collect all items while we have the lock
                                    scripts_lock
                                        .iter()
                                        .filter_map(|(id, state)| {
                                            state.last_output.as_ref().map(|out| {
                                                Self::create_item(id, out)
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                };

                                // Send ALL items to preserve other scripts
                                ctx.send_items("scripts", all_items);
                            }
                            Ok(None) => {
                                tracing::info!("Watch script {} ended", script_id);
                                break;
                            }
                            Err(e) => {
                                tracing::warn!("Error reading from watch script {}: {}", script_id, e);
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

}

#[async_trait]
impl Module for ScriptsModule {
    fn name(&self) -> &str {
        "scripts"
    }

    fn enabled(&self) -> bool {
        // Enabled if there are any enabled scripts
        self.scripts
            .try_read()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        let scripts = self.scripts.read().await;

        if scripts.is_empty() {
            tracing::info!("No enabled scripts configured");
            return;
        }

        // Collect script info before spawning tasks
        let script_info: Vec<_> = scripts
            .iter()
            .map(|(id, state)| {
                (
                    id.clone(),
                    state.config.path.clone(),
                    state.config.mode,
                    state.config.interval_seconds,
                    state.config.icon.clone(),
                )
            })
            .collect();
        drop(scripts);

        // First pass: run all non-watch scripts to populate their cached output
        for (script_id, _path, mode, _interval_secs, _icon) in &script_info {
            match mode {
                ScriptMode::Once | ScriptMode::Interval | ScriptMode::OnConnect => {
                    // Run script to populate cached output
                    let _ = self.update_script(script_id).await;
                }
                ScriptMode::Watch => {
                    // Watch scripts are started separately below
                }
            }
        }

        // Send all initial items together
        let initial_items = self.get_all_items().await;
        if !initial_items.is_empty() {
            ctx.send_items("scripts", initial_items);
        }

        // Second pass: start watch scripts (they'll send updates including all items)
        for (script_id, path, mode, _interval_secs, icon) in script_info {
            if mode == ScriptMode::Watch {
                self.start_watch_script(script_id, path, icon, ctx.clone())
                    .await;
            }
        }

        // Main loop to handle interval scripts
        let scripts = self.scripts.read().await;
        let interval_scripts: Vec<_> = scripts
            .iter()
            .filter(|(_, state)| state.config.mode == ScriptMode::Interval)
            .map(|(id, state)| (id.clone(), state.config.interval_seconds))
            .collect();
        drop(scripts);

        if interval_scripts.is_empty() {
            // No interval scripts, just wait for cancellation
            ctx.cancelled().await;
            return;
        }

        // Simple approach: use the shortest interval and update all interval scripts
        let min_interval = interval_scripts
            .iter()
            .map(|(_, i)| *i)
            .min()
            .unwrap_or(30);

        let poll_interval = Duration::from_secs(min_interval);
        let mut elapsed_secs: u64 = 0;

        loop {
            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(poll_interval) => {
                    elapsed_secs += min_interval;

                    // Update scripts whose interval has elapsed
                    let mut updated_items = Vec::new();
                    for (script_id, interval) in &interval_scripts {
                        if elapsed_secs % interval == 0 {
                            if let Some(item) = self.update_script(script_id).await {
                                updated_items.push(item);
                            }
                        }
                    }

                    if !updated_items.is_empty() {
                        // Send all items together
                        let all_items = self.get_all_items().await;
                        ctx.send_items("scripts", all_items);
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        // Kill any watch child processes
        let mut scripts = self.scripts.write().await;
        for (id, state) in scripts.iter_mut() {
            if let Some(mut child) = state.watch_child.take() {
                tracing::debug!("Killing watch script {}", id);
                let _ = child.kill().await;
            }
        }
        tracing::info!("Scripts module stopped");
    }

    async fn invoke_action(&self, item_id: &str, action_id: &str, _x: i32, _y: i32) {
        // item_id format: "scripts:{script_id}"
        let parts: Vec<&str> = item_id.splitn(2, ':').collect();
        if parts.len() != 2 {
            tracing::warn!("Invalid item_id format: {}", item_id);
            return;
        }
        let script_id = parts[1];

        // Look up the action command
        let key = format!("{}:{}", script_id, action_id);
        let command = {
            let commands = self.action_commands.read().await;
            commands.get(&key).cloned()
        };

        if let Some(cmd) = command {
            tracing::debug!("Executing action {} for script {}: {}", action_id, script_id, cmd);

            // Execute the action command
            let result = Command::new("sh")
                .args(["-c", &cmd])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            match result {
                Ok(mut child) => {
                    // Don't wait for the child, let it run in background
                    tokio::spawn(async move {
                        let _ = child.wait().await;
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to execute action command: {}", e);
                }
            }
        } else {
            tracing::debug!("No command found for action {} on script {}", action_id, script_id);
        }
    }

    async fn reload_config(&self, _config: &crate::config::Config) -> bool {
        // For now, just re-run on_connect scripts
        // A full implementation would compare configs and restart changed scripts
        let scripts = self.scripts.read().await;

        for (script_id, state) in scripts.iter() {
            if state.config.mode == ScriptMode::OnConnect {
                tracing::debug!("Re-running on_connect script {} due to config reload", script_id);
                // Note: We can't call update_script here because we hold a read lock
                // This would need to be restructured for full support
            }
        }

        true
    }
}
