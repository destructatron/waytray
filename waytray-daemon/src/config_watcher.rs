//! Config file watcher for hot reloading

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, EventKind};
use tokio::sync::mpsc;

use crate::config::Config as AppConfig;
use crate::modules::ModuleRegistry;

/// Watch the config file and reload modules when it changes
pub async fn watch_config(
    config_path: impl AsRef<Path>,
    registry: Arc<ModuleRegistry>,
) -> anyhow::Result<()> {
    let config_path = config_path.as_ref().to_path_buf();

    // Create a channel for notify events
    let (tx, mut rx) = mpsc::channel(10);

    // Create the watcher
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<notify::Event, notify::Error>| {
            if let Ok(event) = result {
                // Only send on modify events
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    let _ = tx.blocking_send(());
                }
            }
        },
        Config::default().with_poll_interval(Duration::from_secs(2)),
    )?;

    // Watch the config file's parent directory (watching individual files can be flaky)
    if let Some(parent) = config_path.parent() {
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        tracing::info!("Watching config directory: {:?}", parent);
    } else {
        anyhow::bail!("Config path has no parent directory");
    }

    // Keep the watcher alive and process events
    let _config_filename = config_path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    tokio::spawn(async move {
        // Keep watcher alive
        let _watcher = watcher;

        // Debounce: wait a bit after receiving an event before reloading
        // This handles editors that write files in multiple steps
        let mut pending_reload = false;

        loop {
            tokio::select! {
                Some(()) = rx.recv() => {
                    pending_reload = true;
                }
                _ = tokio::time::sleep(Duration::from_millis(500)), if pending_reload => {
                    pending_reload = false;

                    tracing::info!("Config file changed, reloading...");

                    match AppConfig::load() {
                        Ok(new_config) => {
                            registry.reload_config(&new_config).await;
                            tracing::info!("Config reloaded successfully");
                        }
                        Err(e) => {
                            tracing::error!("Failed to reload config: {}", e);
                        }
                    }
                }
            }
        }
    });

    Ok(())
}
