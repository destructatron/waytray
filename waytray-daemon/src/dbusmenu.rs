//! DBusMenu protocol implementation
//!
//! This module implements the com.canonical.dbusmenu interface for fetching
//! and interacting with application menus exposed via D-Bus.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zbus::proxy;
use zbus::zvariant::{OwnedValue, Value};

/// Proxy for communicating with DBusMenu interfaces
#[proxy(interface = "com.canonical.dbusmenu")]
trait DBusMenu {
    /// Get the menu layout starting from a parent item
    ///
    /// Returns (revision, layout) where layout is (id, properties, children)
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<&str>,
    ) -> zbus::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))>;

    /// Send an event to a menu item (e.g., "clicked")
    fn event(
        &self,
        id: i32,
        event_id: &str,
        data: Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Notify the application that a menu item is about to be shown
    /// Returns true if the menu needs to be refreshed
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;
}

/// A menu item parsed from DBusMenu layout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    /// Unique ID for this menu item
    pub id: i32,
    /// Display label (with mnemonics removed)
    pub label: String,
    /// Whether the item is enabled/clickable
    pub enabled: bool,
    /// Whether the item is visible
    pub visible: bool,
    /// Item type: "standard" or "separator"
    pub item_type: String,
    /// Optional icon name from theme
    pub icon_name: Option<String>,
    /// Toggle type: "checkmark", "radio", or empty
    pub toggle_type: Option<String>,
    /// Toggle state: -1 (off), 0 (indeterminate), 1 (on)
    pub toggle_state: i32,
    /// Child menu items (for submenus)
    pub children: Vec<MenuItem>,
}

impl Default for MenuItem {
    fn default() -> Self {
        Self {
            id: 0,
            label: String::new(),
            enabled: true,
            visible: true,
            item_type: "standard".to_string(),
            icon_name: None,
            toggle_type: None,
            toggle_state: -1,
            children: Vec::new(),
        }
    }
}

/// Fetch menu items from a DBusMenu interface
pub async fn fetch_menu(
    connection: &zbus::Connection,
    bus_name: &str,
    menu_path: &str,
) -> anyhow::Result<Vec<MenuItem>> {
    let proxy = DBusMenuProxy::builder(connection)
        .destination(bus_name)?
        .path(menu_path)?
        .build()
        .await?;

    // Call AboutToShow on root to let the app prepare the menu
    let _ = proxy.about_to_show(0).await;

    // Fetch the complete menu layout
    let property_names = vec![
        "label",
        "enabled",
        "visible",
        "type",
        "icon-name",
        "toggle-type",
        "toggle-state",
        "children-display",
    ];

    let (_revision, layout) = proxy.get_layout(0, -1, property_names).await?;

    // Parse the root item and return its children (root itself is not displayed)
    let root = parse_menu_item(layout, 0)?;
    Ok(root.children)
}

/// Maximum menu nesting depth to prevent stack overflow from malicious menus
const MAX_MENU_DEPTH: usize = 10;

/// Activate a menu item by sending a "clicked" event
pub async fn activate_menu_item(
    connection: &zbus::Connection,
    bus_name: &str,
    menu_path: &str,
    item_id: i32,
) -> anyhow::Result<()> {
    let proxy = DBusMenuProxy::builder(connection)
        .destination(bus_name)?
        .path(menu_path)?
        .build()
        .await?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);

    // Send the clicked event with empty data
    proxy
        .event(item_id, "clicked", Value::new(0i32), timestamp)
        .await?;

    Ok(())
}

/// Parse a menu item from the DBusMenu layout structure
fn parse_menu_item(
    layout: (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>),
    depth: usize,
) -> anyhow::Result<MenuItem> {
    if depth > MAX_MENU_DEPTH {
        anyhow::bail!("Menu depth exceeds maximum of {}", MAX_MENU_DEPTH);
    }

    let (id, properties, children_raw) = layout;

    let mut item = MenuItem {
        id,
        ..Default::default()
    };

    // Parse properties
    for (key, value) in properties {
        match key.as_str() {
            "label" => {
                if let Ok(label) = <&str>::try_from(&*value) {
                    // Remove mnemonic underscores (e.g., "_File" -> "File")
                    item.label = label.replace('_', "");
                }
            }
            "enabled" => {
                if let Ok(enabled) = <bool>::try_from(&*value) {
                    item.enabled = enabled;
                }
            }
            "visible" => {
                if let Ok(visible) = <bool>::try_from(&*value) {
                    item.visible = visible;
                }
            }
            "type" => {
                if let Ok(item_type) = <&str>::try_from(&*value) {
                    item.item_type = item_type.to_string();
                }
            }
            "icon-name" => {
                if let Ok(icon_name) = <&str>::try_from(&*value) {
                    if !icon_name.is_empty() {
                        item.icon_name = Some(icon_name.to_string());
                    }
                }
            }
            "toggle-type" => {
                if let Ok(toggle_type) = <&str>::try_from(&*value) {
                    if !toggle_type.is_empty() {
                        item.toggle_type = Some(toggle_type.to_string());
                    }
                }
            }
            "toggle-state" => {
                if let Ok(state) = <i32>::try_from(&*value) {
                    item.toggle_state = state;
                }
            }
            _ => {}
        }
    }

    // Parse children recursively (with depth limit)
    for child_value in children_raw {
        // Each child is a variant containing (id, properties, children)
        if let Ok(child_struct) = <(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)>::try_from(
            child_value.clone(),
        ) {
            if let Ok(child_item) = parse_menu_item(child_struct, depth + 1) {
                // Only include visible items
                if child_item.visible {
                    item.children.push(child_item);
                }
            }
        }
    }

    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_item_default() {
        let item = MenuItem::default();
        assert_eq!(item.id, 0);
        assert!(item.enabled);
        assert!(item.visible);
        assert_eq!(item.item_type, "standard");
        assert_eq!(item.toggle_state, -1);
    }

    #[test]
    fn test_label_mnemonic_removal() {
        let label = "_File".replace('_', "");
        assert_eq!(label, "File");

        let label = "Save _As...".replace('_', "");
        assert_eq!(label, "Save As...");
    }
}
