//! WayTray Daemon
//!
//! The daemon component of WayTray that caches system tray items and provides
//! a D-Bus interface for clients.

use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use zbus::connection::Connection;

use waytray_daemon::cache::ItemCache;
use waytray_daemon::dbus_service;
use waytray_daemon::host::{self, Host};
use waytray_daemon::watcher::{self, WatcherState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting WayTray daemon");

    // Connect to the session bus
    let connection = Connection::session().await?;
    tracing::info!("Connected to session D-Bus");

    // Create shared state
    let watcher_state = WatcherState::new();
    let cache = ItemCache::new();

    // Start our watcher if no external one exists
    let _owns_watcher = watcher::start_watcher(&connection, watcher_state.clone()).await?;

    // Create and start the host
    let host = Arc::new(Host::new(connection.clone(), cache.clone()).await?);
    host.start().await?;

    // Watch for D-Bus name changes to detect items disappearing
    host::watch_name_changes(connection.clone(), cache.clone()).await?;

    // Start the daemon D-Bus service for clients
    dbus_service::start_service(&connection, cache.clone(), host.clone()).await?;

    tracing::info!("WayTray daemon is running");

    // Keep running until interrupted
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutting down WayTray daemon");
    Ok(())
}
