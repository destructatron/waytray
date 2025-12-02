//! StatusNotifierHost implementation
//!
//! The host is responsible for receiving tray items and caching their data.
//! It connects to the StatusNotifierWatcher (either external or our own) and
//! listens for item registration/unregistration signals.

use std::sync::Arc;
use futures::StreamExt;
use zbus::connection::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::WellKnownName;
use zbus::proxy;
use zbus::zvariant::{OwnedValue, Value};

use crate::cache::ItemCache;
use crate::dbus::HOST_BUS_NAME_PREFIX;
use crate::{ItemCategory, ItemStatus, TrayItem};

/// Proxy for communicating with the StatusNotifierWatcher
#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_host(&self, service: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn status_notifier_item_registered(&self, service: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn status_notifier_item_unregistered(&self, service: String) -> zbus::Result<()>;
}

/// Proxy for communicating with individual StatusNotifierItems
#[proxy(interface = "org.kde.StatusNotifierItem")]
trait StatusNotifierItem {
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn scroll(&self, delta: i32, orientation: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn category(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;

    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> zbus::Result<OwnedValue>;

    #[zbus(property)]
    fn menu(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    #[zbus(property)]
    fn item_is_menu(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn new_title(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_icon(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_status(&self, status: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_tool_tip(&self) -> zbus::Result<()>;
}

/// The StatusNotifierHost that manages tray items
pub struct Host {
    connection: Connection,
    cache: Arc<ItemCache>,
    host_name: String,
}

impl Host {
    /// Create and register a new StatusNotifierHost
    pub async fn new(connection: Connection, cache: Arc<ItemCache>) -> anyhow::Result<Self> {
        let pid = std::process::id();
        let host_name = format!("{}-{}", HOST_BUS_NAME_PREFIX, pid);

        // Request our host bus name
        let bus_name: WellKnownName = host_name.clone().try_into()?;
        connection.request_name(bus_name).await?;

        let host = Self {
            connection,
            cache,
            host_name,
        };

        // Register with the watcher
        host.register_with_watcher().await?;

        Ok(host)
    }

    /// Register ourselves with the StatusNotifierWatcher
    async fn register_with_watcher(&self) -> anyhow::Result<()> {
        let watcher = StatusNotifierWatcherProxy::new(&self.connection).await?;
        watcher.register_status_notifier_host(&self.host_name).await?;
        tracing::info!("Registered as StatusNotifierHost: {}", self.host_name);
        Ok(())
    }

    /// Start listening for item events and process existing items
    pub async fn start(&self) -> anyhow::Result<()> {
        let watcher = StatusNotifierWatcherProxy::new(&self.connection).await?;

        // Get and process existing items
        let existing_items = watcher.registered_status_notifier_items().await?;
        tracing::info!("Found {} existing items", existing_items.len());

        for item_service in existing_items {
            if let Err(e) = self.add_item(&item_service).await {
                tracing::warn!("Failed to add existing item {}: {}", item_service, e);
            }
        }

        // Listen for new items
        let cache = self.cache.clone();
        let connection = self.connection.clone();

        let mut item_registered = watcher.receive_status_notifier_item_registered().await?;
        let mut item_unregistered = watcher.receive_status_notifier_item_unregistered().await?;

        let cache_for_add = cache.clone();
        let connection_for_add = connection.clone();

        // Spawn task to handle item registration
        tokio::spawn(async move {
            while let Some(signal) = item_registered.next().await {
                if let Ok(args) = signal.args() {
                    let service = args.service;
                    tracing::info!("Item registered: {}", service);

                    let host = Host {
                        connection: connection_for_add.clone(),
                        cache: cache_for_add.clone(),
                        host_name: String::new(), // Not used for adding items
                    };

                    if let Err(e) = host.add_item(&service).await {
                        tracing::warn!("Failed to add item {}: {}", service, e);
                    }
                }
            }
        });

        // Spawn task to handle item unregistration
        tokio::spawn(async move {
            while let Some(signal) = item_unregistered.next().await {
                if let Ok(args) = signal.args() {
                    let service = args.service;
                    tracing::info!("Item unregistered: {}", service);
                    cache.remove(&service).await;
                }
            }
        });

        Ok(())
    }

    /// Add a new item to the cache
    async fn add_item(&self, service: &str) -> anyhow::Result<()> {
        let (bus_name, object_path) = parse_service_string(service);

        let proxy = StatusNotifierItemProxy::builder(&self.connection)
            .destination(bus_name.clone())?
            .path(object_path.clone())?
            .build()
            .await?;

        // Fetch item properties
        let item = fetch_item_properties(&proxy, service, &bus_name, &object_path).await?;

        // Add to cache
        self.cache.upsert(item).await;

        // Set up signal handlers for property changes
        self.setup_property_signals(
            service.to_string(),
            bus_name.to_string(),
            object_path.to_string(),
        )
        .await?;

        Ok(())
    }

    /// Set up signal handlers for an item's property change signals
    async fn setup_property_signals(
        &self,
        service: String,
        bus_name: String,
        object_path: String,
    ) -> anyhow::Result<()> {
        let connection = self.connection.clone();
        let cache = self.cache.clone();

        // Create a proxy for setting up signal receivers
        let proxy = StatusNotifierItemProxy::builder(&connection)
            .destination(bus_name.as_str())?
            .path(object_path.as_str())?
            .build()
            .await?;

        // Handle NewTitle signal
        let mut new_title = proxy.receive_new_title().await?;
        let cache_title = cache.clone();
        let service_title = service.clone();
        let connection_title = connection.clone();
        let bus_name_title = bus_name.clone();
        let object_path_title = object_path.clone();

        tokio::spawn(async move {
            while new_title.next().await.is_some() {
                if let Ok(proxy) = StatusNotifierItemProxy::builder(&connection_title)
                    .destination(bus_name_title.as_str())
                    .and_then(|b| b.path(object_path_title.as_str()))
                {
                    if let Ok(proxy) = proxy.build().await {
                        if let Ok(title) = proxy.title().await {
                            cache_title.update_title(&service_title, title).await;
                        }
                    }
                }
            }
        });

        // Handle NewIcon signal
        let mut new_icon = proxy.receive_new_icon().await?;
        let cache_icon = cache.clone();
        let service_icon = service.clone();
        let connection_icon = connection.clone();
        let bus_name_icon = bus_name.clone();
        let object_path_icon = object_path.clone();

        tokio::spawn(async move {
            while new_icon.next().await.is_some() {
                if let Ok(proxy) = StatusNotifierItemProxy::builder(&connection_icon)
                    .destination(bus_name_icon.as_str())
                    .and_then(|b| b.path(object_path_icon.as_str()))
                {
                    if let Ok(proxy) = proxy.build().await {
                        let icon_name = proxy.icon_name().await.ok();
                        let (pixmap, width, height) = fetch_icon_pixmap(&proxy).await;
                        cache_icon
                            .update_icon(&service_icon, icon_name, pixmap, width, height)
                            .await;
                    }
                }
            }
        });

        // Handle NewStatus signal
        let mut new_status = proxy.receive_new_status().await?;
        let cache_status = cache.clone();
        let service_status = service.clone();

        tokio::spawn(async move {
            while let Some(signal) = new_status.next().await {
                if let Ok(args) = signal.args() {
                    let status = ItemStatus::from_str(&args.status);
                    cache_status.update_status(&service_status, status).await;
                }
            }
        });

        // Handle NewToolTip signal
        let mut new_tooltip = proxy.receive_new_tool_tip().await?;
        let cache_tooltip = cache.clone();
        let service_tooltip = service.clone();
        let connection_tooltip = connection.clone();
        let bus_name_tooltip = bus_name.clone();
        let object_path_tooltip = object_path.clone();

        tokio::spawn(async move {
            while new_tooltip.next().await.is_some() {
                if let Ok(proxy) = StatusNotifierItemProxy::builder(&connection_tooltip)
                    .destination(bus_name_tooltip.as_str())
                    .and_then(|b| b.path(object_path_tooltip.as_str()))
                {
                    if let Ok(proxy) = proxy.build().await {
                        let tooltip = fetch_tooltip(&proxy).await;
                        cache_tooltip.update_tooltip(&service_tooltip, tooltip).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Activate an item (primary action)
    pub async fn activate_item(&self, id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        let item = self
            .cache
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Item not found: {}", id))?;

        let proxy = StatusNotifierItemProxy::builder(&self.connection)
            .destination(item.bus_name.clone())?
            .path(item.object_path.clone())?
            .build()
            .await?;

        proxy.activate(x, y).await?;
        Ok(())
    }

    /// Secondary activate an item (middle-click action)
    pub async fn secondary_activate_item(&self, id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        let item = self
            .cache
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Item not found: {}", id))?;

        let proxy = StatusNotifierItemProxy::builder(&self.connection)
            .destination(item.bus_name.clone())?
            .path(item.object_path.clone())?
            .build()
            .await?;

        proxy.secondary_activate(x, y).await?;
        Ok(())
    }

    /// Show context menu for an item
    pub async fn context_menu_item(&self, id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        let item = self
            .cache
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Item not found: {}", id))?;

        let proxy = StatusNotifierItemProxy::builder(&self.connection)
            .destination(item.bus_name.clone())?
            .path(item.object_path.clone())?
            .build()
            .await?;

        proxy.context_menu(x, y).await?;
        Ok(())
    }

    /// Scroll on an item
    pub async fn scroll_item(
        &self,
        id: &str,
        delta: i32,
        orientation: &str,
    ) -> anyhow::Result<()> {
        let item = self
            .cache
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Item not found: {}", id))?;

        let proxy = StatusNotifierItemProxy::builder(&self.connection)
            .destination(item.bus_name.clone())?
            .path(item.object_path.clone())?
            .build()
            .await?;

        proxy.scroll(delta, orientation).await?;
        Ok(())
    }

    /// Get a reference to the cache
    pub fn cache(&self) -> &Arc<ItemCache> {
        &self.cache
    }

    /// Get the D-Bus connection
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Parse a service string into (bus_name, object_path)
///
/// Service strings can be in several formats:
/// - `:1.90/StatusNotifierItem` - unique bus name with path
/// - `:1.75/org/ayatana/NotificationItem/spotify_client` - unique bus name with longer path
/// - `org.kde.StatusNotifierItem-1234-1` - well-known name (default path)
/// - `org.kde.StatusNotifierItem-1234-1:/StatusNotifierItem` - well-known name with path
fn parse_service_string(service: &str) -> (String, String) {
    // Check if it starts with a unique bus name (colon followed by digits/dots)
    if service.starts_with(':') {
        // Format: :1.90/path or :1.90:/path
        // Find the first '/' to split bus name from path
        if let Some(slash_pos) = service.find('/') {
            let bus_name = &service[..slash_pos];
            let path = &service[slash_pos..];
            return (bus_name.to_string(), path.to_string());
        }
        // No path, use default
        return (service.to_string(), "/StatusNotifierItem".to_string());
    }

    // Well-known name format
    // Could be "org.kde.StatusNotifierItem-1234-1" or "org.kde.StatusNotifierItem-1234-1:/path"
    if let Some((bus_name, path)) = service.split_once(":/") {
        (bus_name.to_string(), format!("/{}", path))
    } else if let Some(slash_pos) = service.find('/') {
        // Format: bus_name/path (without colon separator)
        let bus_name = &service[..slash_pos];
        let path = &service[slash_pos..];
        (bus_name.to_string(), path.to_string())
    } else {
        // Just a bus name, use default path
        (service.to_string(), "/StatusNotifierItem".to_string())
    }
}

/// Fetch all properties from an item
async fn fetch_item_properties(
    proxy: &StatusNotifierItemProxy<'_>,
    service: &str,
    bus_name: &str,
    object_path: &str,
) -> anyhow::Result<TrayItem> {
    let id = proxy.id().await.unwrap_or_else(|_| service.to_string());
    let title = proxy.title().await.unwrap_or_else(|_| id.clone());
    let status_str = proxy.status().await.unwrap_or_else(|_| "Active".to_string());
    let category_str = proxy
        .category()
        .await
        .unwrap_or_else(|_| "ApplicationStatus".to_string());
    let icon_name = proxy.icon_name().await.ok().filter(|s| !s.is_empty());

    let (icon_pixmap, icon_width, icon_height) = fetch_icon_pixmap(proxy).await;

    let tooltip = fetch_tooltip(proxy).await;

    let menu_path = proxy.menu().await.ok().map(|p| p.to_string());
    let has_menu = menu_path.is_some();
    let item_is_menu = proxy.item_is_menu().await.unwrap_or(false);

    Ok(TrayItem {
        id: service.to_string(),
        bus_name: bus_name.to_string(),
        object_path: object_path.to_string(),
        title,
        icon_name,
        icon_pixmap,
        icon_width,
        icon_height,
        tooltip,
        status: ItemStatus::from_str(&status_str),
        has_menu,
        menu_path,
        item_is_menu,
        category: ItemCategory::from_str(&category_str),
    })
}

/// Fetch icon pixmap from an item
async fn fetch_icon_pixmap(proxy: &StatusNotifierItemProxy<'_>) -> (Option<Vec<u8>>, u32, u32) {
    match proxy.icon_pixmap().await {
        Ok(pixmaps) if !pixmaps.is_empty() => {
            // Get the largest pixmap
            if let Some((width, height, data)) = pixmaps.into_iter().max_by_key(|(w, h, _)| w * h) {
                (Some(data), width as u32, height as u32)
            } else {
                (None, 0, 0)
            }
        }
        _ => (None, 0, 0),
    }
}

/// Fetch tooltip from an item
async fn fetch_tooltip(proxy: &StatusNotifierItemProxy<'_>) -> Option<String> {
    // ToolTip is a complex type: (icon_name: s, icon_pixmap: a(iiay), title: s, description: s)
    if let Ok(tooltip_value) = proxy.tool_tip().await {
        // Try to extract the title (3rd element) from the struct
        if let Value::Structure(structure) = &*tooltip_value {
            let fields = structure.fields();
            if fields.len() >= 3 {
                if let Value::Str(title) = &fields[2] {
                    let s = title.to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
            // Try description (4th element) if title is empty
            if fields.len() >= 4 {
                if let Value::Str(desc) = &fields[3] {
                    let s = desc.to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
        }
    }
    None
}

/// Watch for D-Bus name owner changes to detect item disappearance
pub async fn watch_name_changes(
    connection: Connection,
    cache: Arc<ItemCache>,
) -> anyhow::Result<()> {
    let dbus = DBusProxy::new(&connection).await?;

    let mut name_owner_changed = dbus.receive_name_owner_changed().await?;

    tokio::spawn(async move {
        while let Some(signal) = name_owner_changed.next().await {
            if let Ok(args) = signal.args() {
                // If new_owner is empty, the name was released
                if args.new_owner.is_none() || args.new_owner.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                    let name = args.name.to_string();
                    // Check if this was a tray item
                    if name.starts_with("org.kde.StatusNotifierItem") {
                        tracing::info!("D-Bus name vanished: {}", name);
                        cache.remove(&name).await;
                    }
                }
            }
        }
    });

    Ok(())
}
