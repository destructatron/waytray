pub mod battery;
pub mod clock;
pub mod network;
pub mod pipewire;
pub mod power_profiles;
pub mod system;
pub mod tray;
pub mod weather;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// A module item that can be displayed in the panel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleItem {
    /// Unique identifier in format "{module_name}:{item_id}"
    pub id: String,
    /// Name of the module this item belongs to
    pub module: String,
    /// Display text for the item
    pub label: String,
    /// Icon name from theme (preferred)
    pub icon_name: Option<String>,
    /// Raw icon data in ARGB32 format (fallback)
    pub icon_pixmap: Option<Vec<u8>>,
    /// Icon width if pixmap is used
    pub icon_width: u32,
    /// Icon height if pixmap is used
    pub icon_height: u32,
    /// Tooltip text
    pub tooltip: Option<String>,
    /// Available actions for this item
    pub actions: Vec<ItemAction>,
}

impl ModuleItem {
    pub fn new(module: &str, item_id: &str, label: &str) -> Self {
        Self {
            id: format!("{}:{}", module, item_id),
            module: module.to_string(),
            label: label.to_string(),
            icon_name: None,
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: None,
            actions: Vec::new(),
        }
    }

    pub fn with_icon_name(mut self, icon_name: &str) -> Self {
        self.icon_name = Some(icon_name.to_string());
        self
    }

    pub fn with_tooltip(mut self, tooltip: &str) -> Self {
        self.tooltip = Some(tooltip.to_string());
        self
    }

    pub fn with_action(mut self, action: ItemAction) -> Self {
        self.actions.push(action);
        self
    }
}

/// An action that can be performed on a module item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemAction {
    /// Unique identifier for this action
    pub id: String,
    /// Display label for this action
    pub label: String,
    /// Whether this is the default action (activated on Enter/click)
    pub is_default: bool,
}

impl ItemAction {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            is_default: false,
        }
    }

    pub fn default_action(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            is_default: true,
        }
    }
}

/// Urgency level for notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// Events emitted by modules
#[derive(Debug, Clone)]
pub enum ModuleEvent {
    /// Module's items have been updated
    ItemsUpdated {
        module_name: String,
        items: Vec<ModuleItem>,
    },
    /// Module wants to send a desktop notification
    Notification {
        title: String,
        body: String,
        urgency: Urgency,
    },
    /// Config has been reloaded, clients should refresh
    ConfigReloaded,
}

/// Context provided to modules for communication
pub struct ModuleContext {
    pub event_sender: broadcast::Sender<ModuleEvent>,
}

impl ModuleContext {
    pub fn new(sender: broadcast::Sender<ModuleEvent>) -> Self {
        Self {
            event_sender: sender,
        }
    }

    pub fn send_items(&self, module_name: &str, items: Vec<ModuleItem>) {
        let _ = self.event_sender.send(ModuleEvent::ItemsUpdated {
            module_name: module_name.to_string(),
            items,
        });
    }

    pub fn send_notification(&self, title: &str, body: &str, urgency: Urgency) {
        let _ = self.event_sender.send(ModuleEvent::Notification {
            title: title.to_string(),
            body: body.to_string(),
            urgency,
        });
    }
}

/// Trait for module implementations
#[async_trait::async_trait]
pub trait Module: Send + Sync {
    /// Get the module's name
    fn name(&self) -> &str;

    /// Check if the module is enabled
    fn enabled(&self) -> bool;

    /// Start the module
    async fn start(&self, ctx: Arc<ModuleContext>);

    /// Stop the module
    async fn stop(&self);

    /// Handle an action invocation on an item
    async fn invoke_action(&self, item_id: &str, action_id: &str, x: i32, y: i32);

    /// Get menu items for a module item (only supported by tray module)
    async fn get_menu_items(&self, _item_id: &str) -> anyhow::Result<Vec<crate::dbusmenu::MenuItem>> {
        anyhow::bail!("Menu not supported by this module")
    }

    /// Activate a menu item (only supported by tray module)
    async fn activate_menu_item(&self, _item_id: &str, _menu_item_id: i32) -> anyhow::Result<()> {
        anyhow::bail!("Menu not supported by this module")
    }

    /// Reload module configuration. Returns true if config was accepted.
    /// Default implementation does nothing (module doesn't support hot reload).
    async fn reload_config(&self, _config: &crate::config::Config) -> bool {
        false
    }
}

use crate::notifications::NotificationService;

/// Registry that manages all modules and their items
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn Module>>,
    module_order: RwLock<Vec<String>>,
    items: Arc<RwLock<HashMap<String, Vec<ModuleItem>>>>,
    event_sender: broadcast::Sender<ModuleEvent>,
    notification_service: Arc<NotificationService>,
}

impl ModuleRegistry {
    pub fn new(module_order: Vec<String>, notification_service: NotificationService) -> Self {
        let (sender, _) = broadcast::channel(64);
        Self {
            modules: Vec::new(),
            module_order: RwLock::new(module_order),
            items: Arc::new(RwLock::new(HashMap::new())),
            event_sender: sender,
            notification_service: Arc::new(notification_service),
        }
    }

    /// Add a module to the registry
    pub fn add_module(&mut self, module: Arc<dyn Module>) {
        self.modules.push(module);
    }

    /// Start all enabled modules and begin listening for events
    pub async fn start(&self) {
        let ctx = Arc::new(ModuleContext::new(self.event_sender.clone()));

        // Start all enabled modules
        for module in &self.modules {
            if module.enabled() {
                tracing::info!("Starting module: {}", module.name());
                let module = module.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    module.start(ctx).await;
                });
            }
        }

        // Listen for module events and update items
        let items = self.items.clone();
        let notification_service = self.notification_service.clone();
        let mut receiver = self.event_sender.subscribe();

        tokio::spawn(async move {
            while let Ok(event) = receiver.recv().await {
                match event {
                    ModuleEvent::ItemsUpdated { module_name, items: new_items } => {
                        let mut items_lock = items.write().await;
                        items_lock.insert(module_name.clone(), new_items);
                        tracing::debug!("Updated items for module: {}", module_name);
                    }
                    ModuleEvent::Notification { title, body, urgency } => {
                        tracing::debug!(
                            "Sending notification: {} - {} ({:?})",
                            title,
                            body,
                            urgency
                        );
                        notification_service.send(&title, &body, urgency);
                    }
                    ModuleEvent::ConfigReloaded => {
                        // Handled by D-Bus service, nothing to do here
                    }
                }
            }
        });
    }

    /// Get all items from all modules, ordered by module_order
    pub async fn get_all_items(&self) -> Vec<ModuleItem> {
        let items_lock = self.items.read().await;
        let order_lock = self.module_order.read().await;
        let mut all_items = Vec::new();

        // Add items in order
        for module_name in order_lock.iter() {
            if let Some(module_items) = items_lock.get(module_name) {
                all_items.extend(module_items.clone());
            }
        }

        // Add items from modules not in the order list
        for (module_name, module_items) in items_lock.iter() {
            if !order_lock.contains(module_name) {
                all_items.extend(module_items.clone());
            }
        }

        all_items
    }

    /// Get items from a specific module
    pub async fn get_module_items(&self, module_name: &str) -> Vec<ModuleItem> {
        let items_lock = self.items.read().await;
        items_lock
            .get(module_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the event sender for subscribing to changes
    pub fn subscribe(&self) -> broadcast::Receiver<ModuleEvent> {
        self.event_sender.subscribe()
    }

    /// Invoke an action on an item
    pub async fn invoke_action(&self, item_id: &str, action_id: &str, x: i32, y: i32) {
        // Parse module name from item_id (format: "module:item")
        let parts: Vec<&str> = item_id.splitn(2, ':').collect();
        if parts.len() != 2 {
            tracing::warn!("Invalid item_id format: {}", item_id);
            return;
        }
        let module_name = parts[0];

        // Find the module and invoke the action
        for module in &self.modules {
            if module.name() == module_name {
                module.invoke_action(item_id, action_id, x, y).await;
                return;
            }
        }

        tracing::warn!("Module not found for item: {}", item_id);
    }

    /// Get menu items for a module item
    pub async fn get_menu_items(&self, item_id: &str) -> anyhow::Result<Vec<crate::dbusmenu::MenuItem>> {
        // Parse module name from item_id (format: "module:item")
        let parts: Vec<&str> = item_id.splitn(2, ':').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid item_id format: {}", item_id);
        }
        let module_name = parts[0];

        // Find the module and get menu items
        for module in &self.modules {
            if module.name() == module_name {
                return module.get_menu_items(item_id).await;
            }
        }

        anyhow::bail!("Module not found for item: {}", item_id)
    }

    /// Activate a menu item
    pub async fn activate_menu_item(&self, item_id: &str, menu_item_id: i32) -> anyhow::Result<()> {
        // Parse module name from item_id (format: "module:item")
        let parts: Vec<&str> = item_id.splitn(2, ':').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid item_id format: {}", item_id);
        }
        let module_name = parts[0];

        // Find the module and activate the menu item
        for module in &self.modules {
            if module.name() == module_name {
                return module.activate_menu_item(item_id, menu_item_id).await;
            }
        }

        anyhow::bail!("Module not found for item: {}", item_id)
    }

    /// Get list of registered modules
    pub fn get_modules(&self) -> Vec<ModuleInfo> {
        self.modules
            .iter()
            .map(|m| ModuleInfo {
                name: m.name().to_string(),
                enabled: m.enabled(),
            })
            .collect()
    }

    /// Reload configuration for all modules
    pub async fn reload_config(&self, config: &crate::config::Config) {
        // Update module order
        let new_order = config.module_order();
        {
            let mut order_lock = self.module_order.write().await;
            *order_lock = new_order;
            tracing::info!("Updated module order");
        }

        // Reload each module's config
        for module in &self.modules {
            let name = module.name();
            if module.reload_config(config).await {
                tracing::info!("Reloaded config for module: {}", name);
            }
        }

        // Notify clients that config was reloaded so they refresh
        let _ = self.event_sender.send(ModuleEvent::ConfigReloaded);
    }
}

/// Information about a registered module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub enabled: bool,
}
