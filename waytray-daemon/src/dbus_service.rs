//! D-Bus service for client communication
//!
//! This module implements the org.waytray.Daemon interface that clients use
//! to query tray items and trigger actions.
//!
//! The interface supports both the legacy tray-only API (for backwards compatibility)
//! and the new module-based API.

use std::sync::Arc;
use zbus::interface;
use zbus::names::WellKnownName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Type;

use crate::cache::ItemCache;
use crate::host::Host;
use crate::modules::{ItemAction, ModuleEvent, ModuleInfo, ModuleItem, ModuleRegistry};
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

/// Serializable version of ModuleItem for D-Bus transport
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Type)]
pub struct ModuleItemDto {
    pub id: String,
    pub module: String,
    pub label: String,
    pub icon_name: String,
    pub icon_pixmap: Vec<u8>,
    pub icon_width: u32,
    pub icon_height: u32,
    pub tooltip: String,
    pub actions: Vec<ItemActionDto>,
}

/// Serializable version of ItemAction for D-Bus transport
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Type)]
pub struct ItemActionDto {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

impl From<ModuleItem> for ModuleItemDto {
    fn from(item: ModuleItem) -> Self {
        Self {
            id: item.id,
            module: item.module,
            label: item.label,
            icon_name: item.icon_name.unwrap_or_default(),
            icon_pixmap: item.icon_pixmap.unwrap_or_default(),
            icon_width: item.icon_width,
            icon_height: item.icon_height,
            tooltip: item.tooltip.unwrap_or_default(),
            actions: item.actions.into_iter().map(ItemActionDto::from).collect(),
        }
    }
}

impl From<ModuleItemDto> for ModuleItem {
    fn from(dto: ModuleItemDto) -> Self {
        Self {
            id: dto.id,
            module: dto.module,
            label: dto.label,
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
            actions: dto.actions.into_iter().map(ItemAction::from).collect(),
        }
    }
}

impl From<ItemAction> for ItemActionDto {
    fn from(action: ItemAction) -> Self {
        Self {
            id: action.id,
            label: action.label,
            is_default: action.is_default,
        }
    }
}

impl From<ItemActionDto> for ItemAction {
    fn from(dto: ItemActionDto) -> Self {
        Self {
            id: dto.id,
            label: dto.label,
            is_default: dto.is_default,
        }
    }
}

/// Serializable version of ModuleInfo for D-Bus transport
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Type)]
pub struct ModuleInfoDto {
    pub name: String,
    pub enabled: bool,
}

impl From<ModuleInfo> for ModuleInfoDto {
    fn from(info: ModuleInfo) -> Self {
        Self {
            name: info.name,
            enabled: info.enabled,
        }
    }
}

impl From<ModuleInfoDto> for ModuleInfo {
    fn from(dto: ModuleInfoDto) -> Self {
        Self {
            name: dto.name,
            enabled: dto.enabled,
        }
    }
}

/// The main daemon D-Bus service
pub struct DaemonService {
    /// Legacy cache for backwards compatibility
    cache: Option<Arc<ItemCache>>,
    /// Legacy host for backwards compatibility
    host: Option<Arc<Host>>,
    /// Module registry for new API
    registry: Option<Arc<ModuleRegistry>>,
}

impl DaemonService {
    /// Create a new service with legacy tray-only support
    pub fn new(cache: Arc<ItemCache>, host: Arc<Host>) -> Self {
        Self {
            cache: Some(cache),
            host: Some(host),
            registry: None,
        }
    }

    /// Create a new service with module support
    pub fn with_registry(registry: Arc<ModuleRegistry>) -> Self {
        Self {
            cache: None,
            host: None,
            registry: Some(registry),
        }
    }
}

#[interface(name = "org.waytray.Daemon")]
impl DaemonService {
    // =========================================================================
    // Legacy API (backwards compatible with old clients)
    // =========================================================================

    /// Get all registered tray items (legacy API)
    async fn get_items(&self) -> Vec<TrayItemDto> {
        if let Some(ref cache) = self.cache {
            cache
                .get_all()
                .await
                .into_iter()
                .map(TrayItemDto::from)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get a single item by ID (legacy API)
    async fn get_item(&self, item_id: &str) -> zbus::fdo::Result<TrayItemDto> {
        if let Some(ref cache) = self.cache {
            cache
                .get(item_id)
                .await
                .map(TrayItemDto::from)
                .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("Item not found: {}", item_id)))
        } else {
            Err(zbus::fdo::Error::InvalidArgs("Legacy API not available".to_string()))
        }
    }

    /// Activate an item (legacy API - primary action, typically left-click)
    async fn activate(&self, item_id: &str, x: i32, y: i32) -> zbus::fdo::Result<()> {
        tracing::debug!("Activate called for item: {} at ({}, {})", item_id, x, y);

        if let Some(ref host) = self.host {
            host
                .activate_item(item_id, x, y)
                .await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
        } else {
            Err(zbus::fdo::Error::InvalidArgs("Legacy API not available".to_string()))
        }
    }

    /// Secondary activate an item (legacy API - typically middle-click)
    async fn secondary_activate(&self, item_id: &str, x: i32, y: i32) -> zbus::fdo::Result<()> {
        tracing::debug!(
            "SecondaryActivate called for item: {} at ({}, {})",
            item_id,
            x,
            y
        );

        if let Some(ref host) = self.host {
            host
                .secondary_activate_item(item_id, x, y)
                .await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
        } else {
            Err(zbus::fdo::Error::InvalidArgs("Legacy API not available".to_string()))
        }
    }

    /// Show context menu for an item (legacy API)
    async fn context_menu(&self, item_id: &str, x: i32, y: i32) -> zbus::fdo::Result<()> {
        tracing::debug!(
            "ContextMenu called for item: {} at ({}, {})",
            item_id,
            x,
            y
        );

        if let Some(ref host) = self.host {
            host
                .context_menu_item(item_id, x, y)
                .await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
        } else {
            Err(zbus::fdo::Error::InvalidArgs("Legacy API not available".to_string()))
        }
    }

    /// Scroll on an item (legacy API)
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

        if let Some(ref host) = self.host {
            host
                .scroll_item(item_id, delta, orientation)
                .await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
        } else {
            Err(zbus::fdo::Error::InvalidArgs("Legacy API not available".to_string()))
        }
    }

    /// Get the number of registered items (legacy API)
    async fn item_count(&self) -> u32 {
        if let Some(ref cache) = self.cache {
            cache.len().await as u32
        } else if let Some(ref registry) = self.registry {
            registry.get_all_items().await.len() as u32
        } else {
            0
        }
    }

    /// Signal emitted when items change (added, removed, or updated)
    #[zbus(signal)]
    pub async fn items_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    // =========================================================================
    // New Module API
    // =========================================================================

    /// Get all items from all modules
    async fn get_all_module_items(&self) -> Vec<ModuleItemDto> {
        if let Some(ref registry) = self.registry {
            registry
                .get_all_items()
                .await
                .into_iter()
                .map(ModuleItemDto::from)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get items from a specific module
    async fn get_module_items(&self, module_name: &str) -> Vec<ModuleItemDto> {
        if let Some(ref registry) = self.registry {
            registry
                .get_module_items(module_name)
                .await
                .into_iter()
                .map(ModuleItemDto::from)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get list of registered modules
    async fn get_modules(&self) -> Vec<ModuleInfoDto> {
        if let Some(ref registry) = self.registry {
            registry
                .get_modules()
                .into_iter()
                .map(ModuleInfoDto::from)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Invoke an action on a module item
    async fn invoke_action(
        &self,
        item_id: &str,
        action_id: &str,
        x: i32,
        y: i32,
    ) -> zbus::fdo::Result<()> {
        tracing::debug!(
            "InvokeAction called: item={}, action={}, at ({}, {})",
            item_id,
            action_id,
            x,
            y
        );

        if let Some(ref registry) = self.registry {
            registry.invoke_action(item_id, action_id, x, y).await;
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs("Module API not available".to_string()))
        }
    }

    /// Signal emitted when module items change
    #[zbus(signal)]
    pub async fn module_items_changed(
        emitter: &SignalEmitter<'_>,
        module_name: &str,
    ) -> zbus::Result<()>;
}

/// Start the daemon D-Bus service (legacy API)
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

/// Start the daemon D-Bus service with module support
pub async fn start_service_with_registry(
    connection: &zbus::Connection,
    registry: Arc<ModuleRegistry>,
) -> anyhow::Result<()> {
    let service = DaemonService::with_registry(registry.clone());

    // Register the interface
    connection
        .object_server()
        .at(crate::dbus::DAEMON_OBJECT_PATH, service)
        .await?;

    // Request the well-known name
    let bus_name: WellKnownName = crate::dbus::DAEMON_BUS_NAME.try_into()?;
    connection.request_name(bus_name).await?;

    tracing::info!(
        "Started D-Bus service with modules: {} at {}",
        crate::dbus::DAEMON_BUS_NAME,
        crate::dbus::DAEMON_OBJECT_PATH
    );

    // Start a task to emit signals when module items change
    let connection_clone = connection.clone();
    let mut rx = registry.subscribe();

    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                ModuleEvent::ItemsUpdated { module_name, .. } => {
                    // Emit the module-specific signal
                    if let Ok(iface_ref) = connection_clone
                        .object_server()
                        .interface::<_, DaemonService>(crate::dbus::DAEMON_OBJECT_PATH)
                        .await
                    {
                        if let Err(e) = DaemonService::module_items_changed(
                            iface_ref.signal_emitter(),
                            &module_name,
                        )
                        .await
                        {
                            tracing::warn!("Failed to emit ModuleItemsChanged signal: {}", e);
                        }
                        // Also emit the legacy signal for backwards compatibility
                        if let Err(e) = DaemonService::items_changed(iface_ref.signal_emitter()).await {
                            tracing::warn!("Failed to emit ItemsChanged signal: {}", e);
                        }
                    }
                }
                ModuleEvent::ConfigReloaded => {
                    // Emit signal so clients refresh with new order
                    if let Ok(iface_ref) = connection_clone
                        .object_server()
                        .interface::<_, DaemonService>(crate::dbus::DAEMON_OBJECT_PATH)
                        .await
                    {
                        if let Err(e) = DaemonService::items_changed(iface_ref.signal_emitter()).await {
                            tracing::warn!("Failed to emit ItemsChanged signal: {}", e);
                        }
                        tracing::debug!("Emitted ItemsChanged signal after config reload");
                    }
                }
                ModuleEvent::Notification { .. } => {
                    // Notifications are handled by the registry, not D-Bus
                }
            }
        }
    });

    Ok(())
}
