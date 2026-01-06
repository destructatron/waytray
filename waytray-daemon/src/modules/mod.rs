pub mod battery;
pub mod clock;
pub mod gpu;
pub mod network;
pub mod pipewire;
pub mod privacy;
pub mod power_profiles;
pub mod scripts;
pub mod system;
pub mod tray;
pub mod weather;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;

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

/// Context provided to modules for communication and lifecycle management
pub struct ModuleContext {
    pub event_sender: broadcast::Sender<ModuleEvent>,
    cancellation_token: CancellationToken,
}

impl ModuleContext {
    pub fn new(sender: broadcast::Sender<ModuleEvent>, cancellation_token: CancellationToken) -> Self {
        Self {
            event_sender: sender,
            cancellation_token,
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

    /// Check if the module should stop
    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    /// Get a future that completes when the module should stop
    pub async fn cancelled(&self) {
        self.cancellation_token.cancelled().await
    }

    /// Get a clone of the cancellation token for use in nested tasks
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
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

use crate::config::Config;
use crate::notifications::NotificationService;

/// A running module with its cancellation token
struct RunningModule {
    module: Arc<dyn Module>,
    cancellation_token: CancellationToken,
}

/// Factory function type for creating modules
pub type ModuleFactory = Box<dyn Fn(&Config, &zbus::Connection) -> Option<Arc<dyn Module>> + Send + Sync>;

/// Registry that manages all modules and their items
pub struct ModuleRegistry {
    /// Running modules indexed by name
    running_modules: RwLock<HashMap<String, RunningModule>>,
    /// Module factories indexed by name
    module_factories: HashMap<String, ModuleFactory>,
    /// Module display order
    module_order: RwLock<Vec<String>>,
    /// Cached items from modules
    items: Arc<RwLock<HashMap<String, Vec<ModuleItem>>>>,
    /// Event sender for module communication
    event_sender: broadcast::Sender<ModuleEvent>,
    /// Notification service
    notification_service: Arc<NotificationService>,
    /// D-Bus connection (needed for tray module)
    connection: zbus::Connection,
}

impl ModuleRegistry {
    pub fn new(
        module_order: Vec<String>,
        notification_service: NotificationService,
        connection: zbus::Connection,
    ) -> Self {
        let (sender, _) = broadcast::channel(64);
        Self {
            running_modules: RwLock::new(HashMap::new()),
            module_factories: HashMap::new(),
            module_order: RwLock::new(module_order),
            items: Arc::new(RwLock::new(HashMap::new())),
            event_sender: sender,
            notification_service: Arc::new(notification_service),
            connection,
        }
    }

    /// Register a module factory
    pub fn register_factory(&mut self, name: &str, factory: ModuleFactory) {
        self.module_factories.insert(name.to_string(), factory);
    }

    /// Start the event listener and initial modules based on config
    pub async fn start(&self, config: &Config) {
        // Start the event listener
        self.start_event_listener();

        // Start modules based on config
        self.sync_modules_with_config(config).await;
    }

    /// Start the event listener that handles module events
    fn start_event_listener(&self) {
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

    /// Start a single module by name
    async fn start_module(&self, name: &str, config: &Config) -> bool {
        // Check if already running
        {
            let running = self.running_modules.read().await;
            if running.contains_key(name) {
                tracing::debug!("Module {} is already running", name);
                return false;
            }
        }

        // Create the module using its factory
        let module = match self.module_factories.get(name) {
            Some(factory) => match factory(config, &self.connection) {
                Some(m) => m,
                None => {
                    tracing::debug!("Factory returned None for module {}", name);
                    return false;
                }
            },
            None => {
                tracing::warn!("No factory registered for module {}", name);
                return false;
            }
        };

        // Create cancellation token and context
        let cancellation_token = CancellationToken::new();
        let ctx = Arc::new(ModuleContext::new(
            self.event_sender.clone(),
            cancellation_token.clone(),
        ));

        // Store the running module
        {
            let mut running = self.running_modules.write().await;
            running.insert(
                name.to_string(),
                RunningModule {
                    module: module.clone(),
                    cancellation_token,
                },
            );
        }

        // Start the module in a background task
        let module_name = name.to_string();
        tokio::spawn(async move {
            tracing::info!("Starting module: {}", module_name);
            module.start(ctx).await;
            tracing::info!("Module {} has stopped", module_name);
        });

        true
    }

    /// Stop a single module by name
    async fn stop_module(&self, name: &str) -> bool {
        let running_module = {
            let mut running = self.running_modules.write().await;
            running.remove(name)
        };

        if let Some(rm) = running_module {
            tracing::info!("Stopping module: {}", name);

            // Signal the module to stop
            rm.cancellation_token.cancel();

            // Call the module's stop method for cleanup
            rm.module.stop().await;

            // Clear the module's items
            {
                let mut items = self.items.write().await;
                items.remove(name);
            }

            true
        } else {
            false
        }
    }

    /// Sync running modules with config (start new, stop removed)
    async fn sync_modules_with_config(&self, config: &Config) {
        let enabled_modules = Self::get_enabled_modules(config);

        // Get currently running module names
        let running_names: HashSet<String> = {
            let running = self.running_modules.read().await;
            running.keys().cloned().collect()
        };

        // Stop modules that are no longer enabled
        for name in &running_names {
            if !enabled_modules.contains(name) {
                self.stop_module(name).await;
            }
        }

        // Start modules that are newly enabled
        for name in &enabled_modules {
            if !running_names.contains(name) {
                self.start_module(name, config).await;
            }
        }

        // Reload config for modules that are still running
        {
            let running = self.running_modules.read().await;
            for (name, rm) in running.iter() {
                if enabled_modules.contains(name) {
                    if rm.module.reload_config(config).await {
                        tracing::info!("Reloaded config for module: {}", name);
                    }
                }
            }
        }
    }

    /// Get set of enabled module names from config
    fn get_enabled_modules(config: &Config) -> HashSet<String> {
        let mut enabled = HashSet::new();

        if config.modules.tray.enabled {
            enabled.insert("tray".to_string());
        }
        if let Some(ref c) = config.modules.battery {
            if c.enabled {
                enabled.insert("battery".to_string());
            }
        }
        if let Some(ref c) = config.modules.clock {
            if c.enabled {
                enabled.insert("clock".to_string());
            }
        }
        if let Some(ref c) = config.modules.system {
            if c.enabled {
                enabled.insert("system".to_string());
            }
        }
        if let Some(ref c) = config.modules.network {
            if c.enabled {
                enabled.insert("network".to_string());
            }
        }
        if let Some(ref c) = config.modules.weather {
            if c.enabled {
                enabled.insert("weather".to_string());
            }
        }
        if let Some(ref c) = config.modules.pipewire {
            if c.enabled {
                enabled.insert("pipewire".to_string());
            }
        }
        if let Some(ref c) = config.modules.privacy {
            if c.enabled {
                enabled.insert("privacy".to_string());
            }
        }
        if let Some(ref c) = config.modules.power_profiles {
            if c.enabled {
                enabled.insert("power_profiles".to_string());
            }
        }
        if let Some(ref c) = config.modules.gpu {
            if c.enabled {
                enabled.insert("gpu".to_string());
            }
        }
        // Check if there are any enabled scripts
        if config.modules.scripts.iter().any(|s| s.enabled) {
            enabled.insert("scripts".to_string());
        }

        enabled
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
        let running = self.running_modules.read().await;
        if let Some(rm) = running.get(module_name) {
            rm.module.invoke_action(item_id, action_id, x, y).await;
        } else {
            tracing::warn!("Module not found for item: {}", item_id);
        }
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
        let running = self.running_modules.read().await;
        if let Some(rm) = running.get(module_name) {
            rm.module.get_menu_items(item_id).await
        } else {
            anyhow::bail!("Module not found for item: {}", item_id)
        }
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
        let running = self.running_modules.read().await;
        if let Some(rm) = running.get(module_name) {
            rm.module.activate_menu_item(item_id, menu_item_id).await
        } else {
            anyhow::bail!("Module not found for item: {}", item_id)
        }
    }

    /// Get list of registered modules
    pub async fn get_modules(&self) -> Vec<ModuleInfo> {
        let running = self.running_modules.read().await;
        running
            .iter()
            .map(|(name, rm)| ModuleInfo {
                name: name.clone(),
                enabled: rm.module.enabled(),
            })
            .collect()
    }

    /// Reload configuration - sync modules and update order
    pub async fn reload_config(&self, config: &Config) {
        // Update module order
        let new_order = config.module_order();
        {
            let mut order_lock = self.module_order.write().await;
            *order_lock = new_order;
            tracing::info!("Updated module order");
        }

        // Sync modules with new config (start/stop as needed)
        self.sync_modules_with_config(config).await;

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
