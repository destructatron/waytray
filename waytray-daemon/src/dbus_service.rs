//! D-Bus service for client communication
//!
//! This module implements the org.waytray.Daemon interface that clients use
//! to query tray items and trigger actions.

use std::sync::Arc;
use zbus::interface;
use zbus::names::WellKnownName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Type;

use crate::cache::ItemCache;
use crate::host::Host;
use crate::{ItemCategory, ItemStatus, TrayItem};

/// Serializable version of TrayItem for D-Bus transport
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Type)]
pub struct TrayItemDto {
    pub id: String,
    pub bus_name: String,
    pub object_path: String,
    pub title: String,
    pub icon_name: String,
    pub icon_pixmap: Vec<u8>,
    pub icon_width: u32,
    pub icon_height: u32,
    pub tooltip: String,
    pub status: String,
    pub has_menu: bool,
    pub menu_path: String,
    pub item_is_menu: bool,
    pub category: String,
}

impl From<TrayItem> for TrayItemDto {
    fn from(item: TrayItem) -> Self {
        Self {
            id: item.id,
            bus_name: item.bus_name,
            object_path: item.object_path,
            title: item.title,
            icon_name: item.icon_name.unwrap_or_default(),
            icon_pixmap: item.icon_pixmap.unwrap_or_default(),
            icon_width: item.icon_width,
            icon_height: item.icon_height,
            tooltip: item.tooltip.unwrap_or_default(),
            status: item.status.as_str().to_string(),
            has_menu: item.has_menu,
            menu_path: item.menu_path.unwrap_or_default(),
            item_is_menu: item.item_is_menu,
            category: match item.category {
                ItemCategory::ApplicationStatus => "ApplicationStatus",
                ItemCategory::Communications => "Communications",
                ItemCategory::SystemServices => "SystemServices",
                ItemCategory::Hardware => "Hardware",
            }
            .to_string(),
        }
    }
}

impl From<TrayItemDto> for TrayItem {
    fn from(dto: TrayItemDto) -> Self {
        Self {
            id: dto.id,
            bus_name: dto.bus_name,
            object_path: dto.object_path,
            title: dto.title,
            icon_name: if dto.icon_name.is_empty() {
                None
            } else {
                Some(dto.icon_name)
            },
            icon_pixmap: if dto.icon_pixmap.is_empty() {
                None
            } else {
                Some(dto.icon_pixmap)
            },
            icon_width: dto.icon_width,
            icon_height: dto.icon_height,
            tooltip: if dto.tooltip.is_empty() {
                None
            } else {
                Some(dto.tooltip)
            },
            status: ItemStatus::from_str(&dto.status),
            has_menu: dto.has_menu,
            menu_path: if dto.menu_path.is_empty() {
                None
            } else {
                Some(dto.menu_path)
            },
            item_is_menu: dto.item_is_menu,
            category: ItemCategory::from_str(&dto.category),
        }
    }
}

/// The main daemon D-Bus service
pub struct DaemonService {
    cache: Arc<ItemCache>,
    host: Arc<Host>,
}

impl DaemonService {
    pub fn new(cache: Arc<ItemCache>, host: Arc<Host>) -> Self {
        Self { cache, host }
    }
}

#[interface(name = "org.waytray.Daemon")]
impl DaemonService {
    /// Get all registered tray items
    async fn get_items(&self) -> Vec<TrayItemDto> {
        self.cache
            .get_all()
            .await
            .into_iter()
            .map(TrayItemDto::from)
            .collect()
    }

    /// Get a single item by ID
    async fn get_item(&self, item_id: &str) -> zbus::fdo::Result<TrayItemDto> {
        self.cache
            .get(item_id)
            .await
            .map(TrayItemDto::from)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("Item not found: {}", item_id)))
    }

    /// Activate an item (primary action, typically left-click)
    async fn activate(&self, item_id: &str, x: i32, y: i32) -> zbus::fdo::Result<()> {
        tracing::debug!("Activate called for item: {} at ({}, {})", item_id, x, y);

        self.host
            .activate_item(item_id, x, y)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Secondary activate an item (typically middle-click)
    async fn secondary_activate(&self, item_id: &str, x: i32, y: i32) -> zbus::fdo::Result<()> {
        tracing::debug!(
            "SecondaryActivate called for item: {} at ({}, {})",
            item_id,
            x,
            y
        );

        self.host
            .secondary_activate_item(item_id, x, y)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Show context menu for an item
    async fn context_menu(&self, item_id: &str, x: i32, y: i32) -> zbus::fdo::Result<()> {
        tracing::debug!(
            "ContextMenu called for item: {} at ({}, {})",
            item_id,
            x,
            y
        );

        self.host
            .context_menu_item(item_id, x, y)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Scroll on an item
    async fn scroll(
        &self,
        item_id: &str,
        delta: i32,
        orientation: &str,
    ) -> zbus::fdo::Result<()> {
        tracing::debug!(
            "Scroll called for item: {} delta={} orientation={}",
            item_id,
            delta,
            orientation
        );

        self.host
            .scroll_item(item_id, delta, orientation)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Get the number of registered items
    async fn item_count(&self) -> u32 {
        self.cache.len().await as u32
    }

    /// Signal emitted when items change (added, removed, or updated)
    #[zbus(signal)]
    pub async fn items_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Start the daemon D-Bus service
pub async fn start_service(
    connection: &zbus::Connection,
    cache: Arc<ItemCache>,
    host: Arc<Host>,
) -> anyhow::Result<()> {
    let service = DaemonService::new(cache.clone(), host);

    // Register the interface
    connection
        .object_server()
        .at(crate::dbus::DAEMON_OBJECT_PATH, service)
        .await?;

    // Request the well-known name
    let bus_name: WellKnownName = crate::dbus::DAEMON_BUS_NAME.try_into()?;
    connection.request_name(bus_name).await?;

    tracing::info!(
        "Started D-Bus service: {} at {}",
        crate::dbus::DAEMON_BUS_NAME,
        crate::dbus::DAEMON_OBJECT_PATH
    );

    // Start a task to emit ItemsChanged signals when the cache changes
    let connection_clone = connection.clone();
    let mut rx = cache.subscribe();

    tokio::spawn(async move {
        while rx.recv().await.is_ok() {
            // Emit the signal
            if let Ok(iface_ref) = connection_clone
                .object_server()
                .interface::<_, DaemonService>(crate::dbus::DAEMON_OBJECT_PATH)
                .await
            {
                if let Err(e) = DaemonService::items_changed(iface_ref.signal_emitter()).await {
                    tracing::warn!("Failed to emit ItemsChanged signal: {}", e);
                }
            }
        }
    });

    Ok(())
}
