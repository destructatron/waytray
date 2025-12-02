//! D-Bus proxy for communicating with the daemon

use waytray_daemon::dbus_service::TrayItemDto;
use waytray_daemon::TrayItem;
use zbus::proxy;
use zbus::Connection;

/// Proxy for the WayTray daemon D-Bus interface
#[proxy(
    interface = "org.waytray.Daemon",
    default_service = "org.waytray.Daemon",
    default_path = "/org/waytray/Daemon"
)]
trait WayTrayDaemon {
    /// Get all registered tray items
    fn get_items(&self) -> zbus::Result<Vec<TrayItemDto>>;

    /// Get a single item by ID
    fn get_item(&self, item_id: &str) -> zbus::Result<TrayItemDto>;

    /// Activate an item (primary action)
    fn activate(&self, item_id: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Secondary activate an item
    fn secondary_activate(&self, item_id: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Show context menu for an item
    fn context_menu(&self, item_id: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Scroll on an item
    fn scroll(&self, item_id: &str, delta: i32, orientation: &str) -> zbus::Result<()>;

    /// Get the number of registered items
    fn item_count(&self) -> zbus::Result<u32>;

    /// Signal emitted when items change
    #[zbus(signal)]
    fn items_changed(&self) -> zbus::Result<()>;
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

    /// Get all tray items
    pub async fn get_items(&self) -> anyhow::Result<Vec<TrayItem>> {
        let items = self.proxy.get_items().await?;
        Ok(items.into_iter().map(TrayItem::from).collect())
    }

    /// Activate an item
    pub async fn activate(&self, item_id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        self.proxy.activate(item_id, x, y).await?;
        Ok(())
    }

    /// Secondary activate an item
    pub async fn secondary_activate(&self, item_id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        self.proxy.secondary_activate(item_id, x, y).await?;
        Ok(())
    }

    /// Show context menu for an item
    pub async fn context_menu(&self, item_id: &str, x: i32, y: i32) -> anyhow::Result<()> {
        self.proxy.context_menu(item_id, x, y).await?;
        Ok(())
    }

    /// Wait for items changed signal (blocks until signal received)
    pub async fn wait_for_items_changed(&self) -> anyhow::Result<()> {
        use futures::StreamExt;
        let mut stream = self.proxy.receive_items_changed().await?;
        stream.next().await;
        Ok(())
    }
}
