//! D-Bus proxy for communicating with the daemon

use waytray_daemon::dbus_service::{ModuleInfoDto, ModuleItemDto, TrayItemDto};
use waytray_daemon::{ModuleInfo, ModuleItem, TrayItem};
use zbus::proxy;
use zbus::Connection;

/// Proxy for the WayTray daemon D-Bus interface
#[proxy(
    interface = "org.waytray.Daemon",
    default_service = "org.waytray.Daemon",
    default_path = "/org/waytray/Daemon"
)]
trait WayTrayDaemon {
    // =========================================================================
    // Legacy API (backwards compatible)
    // =========================================================================

    /// Get all registered tray items (legacy)
    fn get_items(&self) -> zbus::Result<Vec<TrayItemDto>>;

    /// Get a single item by ID (legacy)
    fn get_item(&self, item_id: &str) -> zbus::Result<TrayItemDto>;

    /// Activate an item (legacy - primary action)
    fn activate(&self, item_id: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Secondary activate an item (legacy)
    fn secondary_activate(&self, item_id: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Show context menu for an item (legacy)
    fn context_menu(&self, item_id: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Scroll on an item (legacy)
    fn scroll(&self, item_id: &str, delta: i32, orientation: &str) -> zbus::Result<()>;

    /// Get the number of registered items
    fn item_count(&self) -> zbus::Result<u32>;

    /// Signal emitted when items change (legacy)
    #[zbus(signal)]
    fn items_changed(&self) -> zbus::Result<()>;

    // =========================================================================
    // New Module API
    // =========================================================================

    /// Get all items from all modules
    fn get_all_module_items(&self) -> zbus::Result<Vec<ModuleItemDto>>;

    /// Get items from a specific module
    fn get_module_items(&self, module_name: &str) -> zbus::Result<Vec<ModuleItemDto>>;

    /// Get list of registered modules
    fn get_modules(&self) -> zbus::Result<Vec<ModuleInfoDto>>;

    /// Invoke an action on a module item
    fn invoke_action(
        &self,
        item_id: &str,
        action_id: &str,
        x: i32,
        y: i32,
    ) -> zbus::Result<()>;

    /// Signal emitted when module items change
    #[zbus(signal)]
    fn module_items_changed(&self, module_name: String) -> zbus::Result<()>;
}

/// Client for communicating with the WayTray daemon
pub struct DaemonClient {
    proxy: WayTrayDaemonProxy<'static>,
}

impl DaemonClient {
    /// Create a new daemon client
    pub async fn new() -> anyhow::Result<Self> {
        let connection = Connection::session().await?;
        let proxy = WayTrayDaemonProxy::new(&connection).await?;
        Ok(Self { proxy })
    }

    // =========================================================================
    // Legacy API
    // =========================================================================

    /// Get all tray items (legacy)
    pub async fn get_items(&self) -> anyhow::Result<Vec<TrayItem>> {
        let items = self.proxy.get_items().await?;
        Ok(items.into_iter().map(TrayItem::from).collect())
    }

    /// Activate an item (legacy)
    pub async fn activate(&self, item_id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        self.proxy.activate(item_id, x, y).await?;
        Ok(())
    }

    /// Secondary activate an item (legacy)
    pub async fn secondary_activate(&self, item_id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        self.proxy.secondary_activate(item_id, x, y).await?;
        Ok(())
    }

    /// Show context menu for an item (legacy)
    pub async fn context_menu(&self, item_id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        self.proxy.context_menu(item_id, x, y).await?;
        Ok(())
    }

    /// Wait for items changed signal (legacy - blocks until signal received)
    pub async fn wait_for_items_changed(&self) -> anyhow::Result<()> {
        use futures::StreamExt;
        let mut stream = self.proxy.receive_items_changed().await?;
        stream.next().await;
        Ok(())
    }

    // =========================================================================
    // New Module API
    // =========================================================================

    /// Get all items from all modules
    pub async fn get_all_module_items(&self) -> anyhow::Result<Vec<ModuleItem>> {
        let items = self.proxy.get_all_module_items().await?;
        Ok(items.into_iter().map(ModuleItem::from).collect())
    }

    /// Get items from a specific module
    pub async fn get_module_items(&self, module_name: &str) -> anyhow::Result<Vec<ModuleItem>> {
        let items = self.proxy.get_module_items(module_name).await?;
        Ok(items.into_iter().map(ModuleItem::from).collect())
    }

    /// Get list of registered modules
    pub async fn get_modules(&self) -> anyhow::Result<Vec<ModuleInfo>> {
        let modules = self.proxy.get_modules().await?;
        Ok(modules.into_iter().map(ModuleInfo::from).collect())
    }

    /// Invoke an action on a module item
    pub async fn invoke_action(
        &self,
        item_id: &str,
        action_id: &str,
        x: i32,
        y: i32,
    ) -> anyhow::Result<()> {
        self.proxy.invoke_action(item_id, action_id, x, y).await?;
        Ok(())
    }

    /// Wait for module items changed signal (blocks until signal received)
    pub async fn wait_for_module_items_changed(&self) -> anyhow::Result<String> {
        use futures::StreamExt;
        let mut stream = self.proxy.receive_module_items_changed().await?;
        if let Some(signal) = stream.next().await {
            let args = signal.args()?;
            return Ok(args.module_name);
        }
        Ok(String::new())
    }
}
