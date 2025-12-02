//! WayTray Daemon
//!
//! The daemon component of WayTray that caches system tray items and provides
//! a D-Bus interface for clients.
//!
//! The daemon now supports a modular architecture configured via TOML.

use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use zbus::connection::Connection;

use waytray_daemon::config::Config;
use waytray_daemon::dbus_service;
use waytray_daemon::modules::battery::BatteryModule;
use waytray_daemon::modules::tray::TrayModule;
use waytray_daemon::modules::ModuleRegistry;
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

    // Load configuration
    let config = Config::load()?;
    tracing::info!("Loaded configuration from {:?}", Config::config_path());

    // Connect to the session bus
    let connection = Connection::session().await?;
    tracing::info!("Connected to session D-Bus");

    // Create shared state for the SNI watcher
    let watcher_state = WatcherState::new();

    // Start our watcher if no external one exists
    let _owns_watcher = watcher::start_watcher(&connection, watcher_state.clone()).await?;

    // Create the module registry with configured order
    let module_order = config.module_order();
    let mut registry = ModuleRegistry::new(module_order);

    // Add the tray module if enabled
    if config.modules.tray.enabled {
        let tray_module = TrayModule::new(
            config.modules.tray.clone(),
            connection.clone(),
        );
        registry.add_module(Arc::new(tray_module));
        tracing::info!("Tray module enabled");
    }

    // Add the battery module if enabled
    if let Some(ref battery_config) = config.modules.battery {
        if battery_config.enabled {
            let battery_module = BatteryModule::new(battery_config.clone());
            registry.add_module(Arc::new(battery_module));
            tracing::info!("Battery module enabled");
        }
    }

    // TODO: Add system module when implemented
    // if let Some(ref system_config) = config.modules.system {
    //     if system_config.enabled {
    //         let system_module = SystemModule::new(system_config.clone());
    //         registry.add_module(Arc::new(system_module));
    //     }
    // }

    // TODO: Add script modules when implemented
    // for script_config in &config.modules.scripts {
    //     let script_module = ScriptModule::new(script_config.clone());
    //     registry.add_module(Arc::new(script_module));
    // }

    // Wrap registry in Arc for sharing
    let registry = Arc::new(registry);

    // Start all modules
    registry.start().await;
    tracing::info!("All modules started");

    // Start the daemon D-Bus service for clients
    dbus_service::start_service_with_registry(&connection, registry.clone()).await?;

    tracing::info!("WayTray daemon is running");

    // Keep running until interrupted
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutting down WayTray daemon");
    Ok(())
}
