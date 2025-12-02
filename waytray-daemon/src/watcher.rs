//! StatusNotifierWatcher implementation
//!
//! The watcher is responsible for tracking all registered StatusNotifierItems and hosts.
//! It implements the org.kde.StatusNotifierWatcher D-Bus interface.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use zbus::connection::Connection;
use zbus::fdo::DBusProxy;
use zbus::interface;
use zbus::message::Header;
use zbus::names::WellKnownName;
use zbus::object_server::SignalEmitter;

use crate::dbus::{WATCHER_BUS_NAME, WATCHER_OBJECT_PATH};

/// Internal state for the watcher
pub struct WatcherState {
    /// Registered StatusNotifierItem service names
    pub registered_items: RwLock<HashSet<String>>,
    /// Whether at least one host is registered
    pub host_registered: RwLock<bool>,
    /// Registered host names (for tracking)
    pub registered_hosts: RwLock<HashSet<String>>,
}

impl WatcherState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            registered_items: RwLock::new(HashSet::new()),
            host_registered: RwLock::new(false),
            registered_hosts: RwLock::new(HashSet::new()),
        })
    }
}

impl Default for WatcherState {
    fn default() -> Self {
        Self {
            registered_items: RwLock::new(HashSet::new()),
            host_registered: RwLock::new(false),
            registered_hosts: RwLock::new(HashSet::new()),
        }
    }
}

/// StatusNotifierWatcher D-Bus interface implementation
pub struct StatusNotifierWatcher {
    state: Arc<WatcherState>,
}

impl StatusNotifierWatcher {
    pub fn new(state: Arc<WatcherState>) -> Self {
        Self { state }
    }
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    /// Register a StatusNotifierItem
    ///
    /// The service parameter can be either:
    /// - A full bus name (e.g., "org.kde.StatusNotifierItem-1234-1")
    /// - An object path (e.g., "/org/ayatana/NotificationItem/steam")
    ///
    /// In the second case, we use the message sender's unique bus name.
    async fn register_status_notifier_item(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        service: &str,
    ) -> zbus::fdo::Result<()> {
        let full_service = if service.starts_with('/') {
            // Object path style - use sender's bus name
            if let Some(sender) = header.sender() {
                format!("{}:{}", sender, service)
            } else {
                return Err(zbus::fdo::Error::InvalidArgs(
                    "Could not determine sender".to_string(),
                ));
            }
        } else {
            // Full bus name style
            service.to_string()
        };

        tracing::info!("Registering StatusNotifierItem: {}", full_service);

        {
            let mut items = self.state.registered_items.write().await;
            items.insert(full_service.clone());
        }

        // Emit signal
        Self::status_notifier_item_registered(&emitter, &full_service).await?;

        Ok(())
    }

    /// Register a StatusNotifierHost
    async fn register_status_notifier_host(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        service: &str,
    ) -> zbus::fdo::Result<()> {
        tracing::info!("Registering StatusNotifierHost: {}", service);

        {
            let mut hosts = self.state.registered_hosts.write().await;
            hosts.insert(service.to_string());
        }

        {
            let mut host_registered = self.state.host_registered.write().await;
            *host_registered = true;
        }

        Self::status_notifier_host_registered(&emitter).await?;

        Ok(())
    }

    /// Get all registered StatusNotifierItems
    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        let items = self.state.registered_items.read().await;
        items.iter().cloned().collect()
    }

    /// Check if at least one host is registered
    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        *self.state.host_registered.read().await
    }

    /// Protocol version
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    /// Signal emitted when a new item is registered
    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    /// Signal emitted when an item is unregistered
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    /// Signal emitted when a host is registered
    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// Signal emitted when a host is unregistered
    #[zbus(signal)]
    async fn status_notifier_host_unregistered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Check if an external watcher already exists on the session bus
pub async fn external_watcher_exists(connection: &Connection) -> bool {
    let dbus = DBusProxy::new(connection).await.ok();
    if let Some(proxy) = dbus {
        proxy.name_has_owner(WATCHER_BUS_NAME.try_into().unwrap()).await.unwrap_or(false)
    } else {
        false
    }
}

/// Start our own StatusNotifierWatcher if no external one exists
pub async fn start_watcher(
    connection: &Connection,
    state: Arc<WatcherState>,
) -> anyhow::Result<bool> {
    if external_watcher_exists(connection).await {
        tracing::info!("External StatusNotifierWatcher found, not starting our own");
        return Ok(false);
    }

    tracing::info!("No external watcher found, starting our own");

    // Register the watcher interface
    let watcher = StatusNotifierWatcher::new(state);
    connection
        .object_server()
        .at(WATCHER_OBJECT_PATH, watcher)
        .await?;

    // Request the well-known name
    let bus_name: WellKnownName = WATCHER_BUS_NAME.try_into()?;
    connection.request_name(bus_name).await?;

    Ok(true)
}

/// Handle item unregistration (called when D-Bus name vanishes)
pub async fn handle_item_unregistered(
    state: &WatcherState,
    connection: &Connection,
    service: &str,
) -> anyhow::Result<()> {
    let removed = {
        let mut items = state.registered_items.write().await;
        items.remove(service)
    };

    if removed {
        tracing::info!("StatusNotifierItem unregistered: {}", service);

        // Emit signal
        let interface_ref = connection
            .object_server()
            .interface::<_, StatusNotifierWatcher>(WATCHER_OBJECT_PATH)
            .await?;

        StatusNotifierWatcher::status_notifier_item_unregistered(
            interface_ref.signal_emitter(),
            service,
        )
        .await?;
    }

    Ok(())
}
