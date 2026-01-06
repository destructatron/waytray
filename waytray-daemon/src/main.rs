//! WayTray Daemon
//!
//! The daemon component of WayTray that caches system tray items and provides
//! a D-Bus interface for clients.
//!
//! The daemon now supports a modular architecture configured via TOML.
//! Modules can be dynamically loaded and unloaded based on configuration changes.

use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use zbus::connection::Connection;

use waytray_daemon::config::Config;
use waytray_daemon::config_watcher;
use waytray_daemon::dbus_service;
use waytray_daemon::modules::battery::BatteryModule;
use waytray_daemon::modules::clock::ClockModule;
use waytray_daemon::modules::gpu::GpuModule;
use waytray_daemon::modules::network::NetworkModule;
use waytray_daemon::modules::pipewire::PipewireModule;
use waytray_daemon::modules::privacy::PrivacyModule;
use waytray_daemon::modules::power_profiles::PowerProfilesModule;
use waytray_daemon::modules::scripts::ScriptsModule;
use waytray_daemon::modules::system::SystemModule;
use waytray_daemon::modules::tray::TrayModule;
use waytray_daemon::modules::weather::WeatherModule;
use waytray_daemon::modules::ModuleRegistry;
use waytray_daemon::notifications::NotificationService;
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

    // Create notification service
    let notification_service = NotificationService::new(
        config.notifications.enabled,
        config.notifications.timeout_ms,
    );

    // Create the module registry with configured order
    let module_order = config.module_order();
    let mut registry = ModuleRegistry::new(module_order, notification_service, connection.clone());

    // Register module factories
    register_module_factories(&mut registry);

    // Wrap registry in Arc for sharing
    let registry = Arc::new(registry);

    // Start all enabled modules
    registry.start(&config).await;
    tracing::info!("All modules started");

    // Start the daemon D-Bus service for clients
    dbus_service::start_service_with_registry(&connection, registry.clone()).await?;

    // Start config file watcher for hot reload
    let config_path = Config::config_path();
    if let Err(e) = config_watcher::watch_config(config_path, registry.clone()).await {
        tracing::warn!("Failed to start config watcher: {}", e);
    }

    tracing::info!("WayTray daemon is running");

    // Keep running until interrupted
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutting down WayTray daemon");
    Ok(())
}

/// Register all module factories with the registry
fn register_module_factories(registry: &mut ModuleRegistry) {
    // Tray module factory
    registry.register_factory(
        "tray",
        Box::new(|config, connection| {
            if config.modules.tray.enabled {
                Some(Arc::new(TrayModule::new(
                    config.modules.tray.clone(),
                    connection.clone(),
                )))
            } else {
                None
            }
        }),
    );

    // Battery module factory
    registry.register_factory(
        "battery",
        Box::new(|config, _connection| {
            config.modules.battery.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(BatteryModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Clock module factory
    registry.register_factory(
        "clock",
        Box::new(|config, _connection| {
            config.modules.clock.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(ClockModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // System module factory
    registry.register_factory(
        "system",
        Box::new(|config, _connection| {
            config.modules.system.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(SystemModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Network module factory
    registry.register_factory(
        "network",
        Box::new(|config, _connection| {
            config.modules.network.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(NetworkModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Weather module factory
    registry.register_factory(
        "weather",
        Box::new(|config, _connection| {
            config.modules.weather.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(WeatherModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Pipewire module factory
    registry.register_factory(
        "pipewire",
        Box::new(|config, _connection| {
            config.modules.pipewire.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(PipewireModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Privacy module factory
    registry.register_factory(
        "privacy",
        Box::new(|config, _connection| {
            config.modules.privacy.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(PrivacyModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Power profiles module factory
    registry.register_factory(
        "power_profiles",
        Box::new(|config, _connection| {
            config.modules.power_profiles.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(PowerProfilesModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // GPU module factory
    registry.register_factory(
        "gpu",
        Box::new(|config, _connection| {
            config.modules.gpu.as_ref().and_then(|c| {
                if c.enabled {
                    Some(Arc::new(GpuModule::new(c.clone())) as Arc<dyn waytray_daemon::modules::Module>)
                } else {
                    None
                }
            })
        }),
    );

    // Scripts module factory
    registry.register_factory(
        "scripts",
        Box::new(|config, _connection| {
            // Check if there are any enabled scripts
            let enabled_scripts: Vec<_> = config
                .modules
                .scripts
                .iter()
                .filter(|s| s.enabled)
                .cloned()
                .collect();

            if enabled_scripts.is_empty() {
                None
            } else {
                Some(Arc::new(ScriptsModule::new(enabled_scripts)) as Arc<dyn waytray_daemon::modules::Module>)
            }
        }),
    );
}
