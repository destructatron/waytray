//! Item cache for storing and managing tray items

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::{CacheEvent, TrayItem};

/// Cache for storing tray items
///
/// This cache is thread-safe and can be shared across async tasks.
/// It provides a broadcast channel for notifying listeners of changes.
pub struct ItemCache {
    items: RwLock<HashMap<String, TrayItem>>,
    change_tx: broadcast::Sender<CacheEvent>,
}

impl ItemCache {
    /// Create a new item cache
    pub fn new() -> Arc<Self> {
        let (change_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            items: RwLock::new(HashMap::new()),
            change_tx,
        })
    }

    /// Subscribe to cache change events
    pub fn subscribe(&self) -> broadcast::Receiver<CacheEvent> {
        self.change_tx.subscribe()
    }

    /// Add or update an item in the cache
    pub async fn upsert(&self, item: TrayItem) {
        let id = item.id.clone();
        let mut items = self.items.write().await;
        let is_new = !items.contains_key(&id);
        items.insert(id.clone(), item);

        let event = if is_new {
            CacheEvent::ItemAdded(id)
        } else {
            CacheEvent::ItemUpdated(id)
        };

        // Ignore send errors (no receivers)
        let _ = self.change_tx.send(event);
    }

    /// Remove an item from the cache
    pub async fn remove(&self, id: &str) -> Option<TrayItem> {
        let mut items = self.items.write().await;
        let removed = items.remove(id);

        if removed.is_some() {
            let _ = self.change_tx.send(CacheEvent::ItemRemoved(id.to_string()));
        }

        removed
    }

    /// Remove all items that have the given bus_name
    ///
    /// This is used when a D-Bus connection disappears (e.g., app crashes)
    /// to clean up any items that were registered with that connection.
    pub async fn remove_by_bus_name(&self, bus_name: &str) -> Vec<TrayItem> {
        let mut items = self.items.write().await;

        // Find all item IDs that match this bus_name
        let ids_to_remove: Vec<String> = items
            .iter()
            .filter(|(_, item)| item.bus_name == bus_name)
            .map(|(id, _)| id.clone())
            .collect();

        let mut removed = Vec::new();
        for id in ids_to_remove {
            if let Some(item) = items.remove(&id) {
                let _ = self.change_tx.send(CacheEvent::ItemRemoved(id));
                removed.push(item);
            }
        }

        removed
    }

    /// Get an item by ID
    pub async fn get(&self, id: &str) -> Option<TrayItem> {
        let items = self.items.read().await;
        items.get(id).cloned()
    }

    /// Get all items
    pub async fn get_all(&self) -> Vec<TrayItem> {
        let items = self.items.read().await;
        items.values().cloned().collect()
    }

    /// Check if an item exists
    pub async fn contains(&self, id: &str) -> bool {
        let items = self.items.read().await;
        items.contains_key(id)
    }

    /// Get the number of items
    pub async fn len(&self) -> usize {
        let items = self.items.read().await;
        items.len()
    }

    /// Check if the cache is empty
    pub async fn is_empty(&self) -> bool {
        let items = self.items.read().await;
        items.is_empty()
    }

    /// Update a specific field of an item
    pub async fn update_title(&self, id: &str, title: String) {
        let mut items = self.items.write().await;
        if let Some(item) = items.get_mut(id) {
            item.title = title;
            let _ = self.change_tx.send(CacheEvent::ItemUpdated(id.to_string()));
        }
    }

    /// Update the status of an item
    pub async fn update_status(&self, id: &str, status: crate::ItemStatus) {
        let mut items = self.items.write().await;
        if let Some(item) = items.get_mut(id) {
            item.status = status;
            let _ = self.change_tx.send(CacheEvent::ItemUpdated(id.to_string()));
        }
    }

    /// Update the icon of an item
    pub async fn update_icon(
        &self,
        id: &str,
        icon_name: Option<String>,
        icon_pixmap: Option<Vec<u8>>,
        width: u32,
        height: u32,
    ) {
        let mut items = self.items.write().await;
        if let Some(item) = items.get_mut(id) {
            item.icon_name = icon_name;
            item.icon_pixmap = icon_pixmap;
            item.icon_width = width;
            item.icon_height = height;
            let _ = self.change_tx.send(CacheEvent::ItemUpdated(id.to_string()));
        }
    }

    /// Update the tooltip of an item
    pub async fn update_tooltip(&self, id: &str, tooltip: Option<String>) {
        let mut items = self.items.write().await;
        if let Some(item) = items.get_mut(id) {
            item.tooltip = tooltip;
            let _ = self.change_tx.send(CacheEvent::ItemUpdated(id.to_string()));
        }
    }

    /// Notify that items have changed (for manual broadcast)
    pub fn notify_changed(&self) {
        // Send a generic update for when we don't know which item changed
        let _ = self.change_tx.send(CacheEvent::ItemUpdated(String::new()));
    }
}

impl Default for ItemCache {
    fn default() -> Self {
        let (change_tx, _) = broadcast::channel(64);
        Self {
            items: RwLock::new(HashMap::new()),
            change_tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ItemStatus;

    fn make_test_item(id: &str) -> TrayItem {
        TrayItem {
            id: id.to_string(),
            bus_name: format!("org.test.{}", id),
            object_path: "/StatusNotifierItem".to_string(),
            title: format!("Test Item {}", id),
            icon_name: Some("test-icon".to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: None,
            status: ItemStatus::Active,
            has_menu: false,
            menu_path: None,
            item_is_menu: false,
            category: crate::ItemCategory::ApplicationStatus,
        }
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let cache = ItemCache::new();

        // Add item
        cache.upsert(make_test_item("1")).await;
        assert_eq!(cache.len().await, 1);
        assert!(cache.contains("1").await);

        // Get item
        let item = cache.get("1").await.unwrap();
        assert_eq!(item.title, "Test Item 1");

        // Update item
        let mut updated = make_test_item("1");
        updated.title = "Updated Title".to_string();
        cache.upsert(updated).await;
        let item = cache.get("1").await.unwrap();
        assert_eq!(item.title, "Updated Title");

        // Remove item
        cache.remove("1").await;
        assert!(cache.is_empty().await);
    }
}
