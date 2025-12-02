//! WayTray Daemon Library
//!
//! This library provides the core types and functionality for the WayTray system tray daemon.
//! It implements the StatusNotifierItem (SNI) host protocol and caches tray items for clients.

pub mod cache;
pub mod dbus_service;
pub mod host;
pub mod watcher;

use serde::{Deserialize, Serialize};

/// Represents a single system tray item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayItem {
    /// Unique identifier for this item (typically bus_name + object_path)
    pub id: String,
    /// D-Bus bus name of the application
    pub bus_name: String,
    /// D-Bus object path for the StatusNotifierItem
    pub object_path: String,
    /// Display title of the item
    pub title: String,
    /// Icon name from freedesktop icon theme (preferred)
    pub icon_name: Option<String>,
    /// Raw ARGB32 icon pixmap data (fallback if icon_name not available)
    pub icon_pixmap: Option<Vec<u8>>,
    /// Width of the icon pixmap
    pub icon_width: u32,
    /// Height of the icon pixmap
    pub icon_height: u32,
    /// Tooltip text
    pub tooltip: Option<String>,
    /// Current status of the item
    pub status: ItemStatus,
    /// Whether the item has a D-Bus menu
    pub has_menu: bool,
    /// D-Bus object path for the menu (if has_menu is true)
    pub menu_path: Option<String>,
    /// Whether this item is menu-only (clicking should show menu, not activate)
    pub item_is_menu: bool,
    /// Category of the item
    pub category: ItemCategory,
}

/// Status of a tray item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ItemStatus {
    /// The item doesn't convey important information and can be hidden
    #[default]
    Passive,
    /// The item is active and should be shown
    Active,
    /// The item needs attention (e.g., new message)
    NeedsAttention,
}

impl ItemStatus {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "active" => ItemStatus::Active,
            "needsattention" | "needs-attention" => ItemStatus::NeedsAttention,
            _ => ItemStatus::Passive,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ItemStatus::Passive => "Passive",
            ItemStatus::Active => "Active",
            ItemStatus::NeedsAttention => "NeedsAttention",
        }
    }
}

/// Category of a tray item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ItemCategory {
    /// Application status or notifications
    #[default]
    ApplicationStatus,
    /// Communication-related (email, chat, etc.)
    Communications,
    /// System services (volume, network, battery)
    SystemServices,
    /// Hardware status (printers, removable media)
    Hardware,
}

impl ItemCategory {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "communications" => ItemCategory::Communications,
            "systemservices" | "system-services" => ItemCategory::SystemServices,
            "hardware" => ItemCategory::Hardware,
            _ => ItemCategory::ApplicationStatus,
        }
    }
}

/// Events emitted by the item cache
#[derive(Debug, Clone)]
pub enum CacheEvent {
    /// A new item was added
    ItemAdded(String),
    /// An item was removed
    ItemRemoved(String),
    /// An item was updated
    ItemUpdated(String),
}

/// D-Bus well-known names and paths
pub mod dbus {
    /// StatusNotifierWatcher well-known name
    pub const WATCHER_BUS_NAME: &str = "org.kde.StatusNotifierWatcher";
    /// StatusNotifierWatcher object path
    pub const WATCHER_OBJECT_PATH: &str = "/StatusNotifierWatcher";

    /// StatusNotifierHost well-known name prefix
    pub const HOST_BUS_NAME_PREFIX: &str = "org.kde.StatusNotifierHost";

    /// WayTray daemon well-known name
    pub const DAEMON_BUS_NAME: &str = "org.waytray.Daemon";
    /// WayTray daemon object path
    pub const DAEMON_OBJECT_PATH: &str = "/org/waytray/Daemon";
}
