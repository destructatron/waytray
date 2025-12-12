//! Main window for the WayTray client
//!
//! Uses a horizontal FlowBox for left/right arrow navigation like KDE's system tray.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::daemon_proxy::DaemonClient;
use crate::menu_popover::MenuPopover;
use crate::module_item::ModuleItemWidget;
use waytray_daemon::ModuleItem;

mod imp {
    use super::*;
    use gtk4::subclass::application_window::ApplicationWindowImpl;
    use gtk4::subclass::widget::WidgetImpl;
    use gtk4::subclass::window::WindowImpl;

    pub struct WayTrayWindow {
        /// Horizontal box containing the items (replaces FlowBox for better a11y)
        pub items_box: gtk4::Box,
        pub scrolled_window: gtk4::ScrolledWindow,
        pub status_label: gtk4::Label,
        pub main_box: gtk4::Box,
        pub client: RefCell<Option<Arc<DaemonClient>>>,
        /// Track if window has ever been focused (for close-on-focus-loss behavior)
        pub has_been_focused: Cell<bool>,
        /// Track number of open popovers (don't close window while popovers are open)
        pub open_popover_count: Cell<u32>,
        /// Temporarily suppress close-on-focus-loss after popover closes
        pub suppress_close_until: Cell<Option<std::time::Instant>>,
        /// Track last keyboard interaction to prevent close during active use
        pub last_keyboard_interaction: Cell<Option<std::time::Instant>>,
        /// Pending close - can be cancelled by keyboard interaction
        pub pending_close_id: Cell<u64>,
    }

    impl Default for WayTrayWindow {
        fn default() -> Self {
            Self {
                items_box: gtk4::Box::new(gtk4::Orientation::Horizontal, 0),
                scrolled_window: gtk4::ScrolledWindow::new(),
                status_label: gtk4::Label::new(Some("Connecting to daemon...")),
                main_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                client: RefCell::new(None),
                has_been_focused: Cell::new(false),
                open_popover_count: Cell::new(0),
                suppress_close_until: Cell::new(None),
                last_keyboard_interaction: Cell::new(None),
                pending_close_id: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WayTrayWindow {
        const NAME: &'static str = "WayTrayWindow";
        type Type = super::WayTrayWindow;
        type ParentType = gtk4::ApplicationWindow;
    }

    impl ObjectImpl for WayTrayWindow {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            // Configure window
            obj.set_title(Some("System Tray"));
            obj.set_default_size(600, 60);
            obj.set_resizable(true);

            // Configure the horizontal items box
            self.items_box.set_orientation(gtk4::Orientation::Horizontal);
            self.items_box.set_spacing(4);

            // Set accessible role for the items container
            self.items_box
                .set_accessible_role(gtk4::AccessibleRole::List);

            // Configure scrolled window for horizontal scrolling only
            self.scrolled_window.set_child(Some(&self.items_box));
            self.scrolled_window.set_vexpand(false);
            self.scrolled_window.set_hexpand(true);
            self.scrolled_window
                .set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);

            // Configure status label
            self.status_label.set_margin_top(12);
            self.status_label.set_margin_bottom(12);
            self.status_label.set_margin_start(12);
            self.status_label.set_margin_end(12);

            // Build layout
            self.main_box.append(&self.status_label);
            self.main_box.append(&self.scrolled_window);

            obj.set_child(Some(&self.main_box));

            // Set up keyboard handling for left/right navigation and escape
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.connect_key_pressed(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, keyval, _keycode, _state| {
                    let imp = obj.imp();
                    // Record keyboard interaction to prevent spurious close-on-focus-loss
                    imp.last_keyboard_interaction.set(Some(std::time::Instant::now()));
                    // Cancel any pending close - user is actively interacting
                    if imp.pending_close_id.get() > 0 {
                        imp.pending_close_id.set(0);
                    }

                    match keyval {
                        gdk::Key::Escape => {
                            obj.close();
                            glib::Propagation::Stop
                        }
                        gdk::Key::Left => {
                            obj.navigate_items(-1);
                            glib::Propagation::Stop
                        }
                        gdk::Key::Right => {
                            obj.navigate_items(1);
                            glib::Propagation::Stop
                        }
                        _ => glib::Propagation::Proceed,
                    }
                }
            ));
            obj.add_controller(key_controller);

            // Close window when focus leaves (but not when using menus/popovers)
            // Only close after the window has been focused at least once (handles
            // compositors like Niri where fullscreen windows prevent initial focus)
            obj.connect_notify_local(Some("is-active"), |window, _| {
                let imp = window.imp();

                if window.is_active() {
                    imp.has_been_focused.set(true);
                    // Cancel any pending close since we're active again
                    imp.pending_close_id.set(0);
                } else if imp.has_been_focused.get() && imp.open_popover_count.get() == 0 {
                    // Check if close is temporarily suppressed (popover just closed)
                    if let Some(until) = imp.suppress_close_until.get() {
                        if std::time::Instant::now() < until {
                            return;
                        }
                    }
                    // Instead of closing immediately, schedule a delayed close.
                    // This allows key events that triggered the focus loss to cancel the close.
                    // The issue: pressing arrow keys can cause is-active to become false
                    // BEFORE the key handler runs, so we need to give the key handler time.
                    let close_id = imp.pending_close_id.get().wrapping_add(1).max(1);
                    imp.pending_close_id.set(close_id);

                    glib::timeout_add_local_once(std::time::Duration::from_millis(150), glib::clone!(
                        #[weak]
                        window,
                        move || {
                            let imp = window.imp();
                            if imp.pending_close_id.get() == close_id && !window.is_active() {
                                window.close();
                            }
                        }
                    ));
                }
            });
        }
    }

    impl WidgetImpl for WayTrayWindow {}
    impl WindowImpl for WayTrayWindow {}
    impl ApplicationWindowImpl for WayTrayWindow {}
}

glib::wrapper! {
    pub struct WayTrayWindow(ObjectSubclass<imp::WayTrayWindow>)
        @extends gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk4::Accessible, gtk4::Buildable,
                    gtk4::ConstraintTarget, gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl WayTrayWindow {
    pub fn new(app: &gtk4::Application) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .build();

        // Initialize connection to daemon
        window.connect_to_daemon();

        window
    }

    /// Connect to the WayTray daemon and start listening for updates
    fn connect_to_daemon(&self) {
        let window = self.clone();

        glib::spawn_future_local(async move {
            match DaemonClient::new().await {
                Ok(client) => {
                    let client = Arc::new(client);
                    window.imp().client.replace(Some(client.clone()));

                    // Fetch initial items
                    window.refresh_items().await;

                    // Listen for changes
                    window.listen_for_changes(client);
                }
                Err(e) => {
                    tracing::error!("Failed to connect to daemon: {}", e);
                    window.show_error(&format!(
                        "Failed to connect to daemon: {}\n\nMake sure waytray-daemon is running.",
                        e
                    ));
                }
            }
        });
    }

    /// Refresh the list of module items
    async fn refresh_items(&self) {
        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        match client.get_all_module_items().await {
            Ok(items) => {
                self.update_items(&items);
            }
            Err(e) => {
                tracing::error!("Failed to get items: {}", e);
                self.show_error(&format!("Failed to get items: {}", e));
            }
        }
    }

    /// Update the displayed items (incremental update to preserve focus)
    fn update_items(&self, items: &[ModuleItem]) {
        let imp = self.imp();

        let was_empty = imp.items_box.first_child().is_none();
        let focused_item_id = self.get_focused_item_id();

        // Build set of new item IDs
        let new_ids: std::collections::HashSet<&str> =
            items.iter().map(|i| i.id.as_str()).collect();

        // Remove widgets for items that no longer exist
        let mut to_remove = Vec::new();
        let mut child = imp.items_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Some(item_widget) = widget.downcast_ref::<ModuleItemWidget>() {
                if let Some(id) = item_widget.item_id() {
                    if !new_ids.contains(id.as_str()) {
                        to_remove.push(id);
                    }
                }
            }
            child = next;
        }

        for id in &to_remove {
            if let Some(widget) = self.find_item_widget(id) {
                imp.items_box.remove(&widget);
            }
        }

        // Update existing items or add new ones in order
        for item in items {
            if let Some(existing_widget) = self.find_item_widget(&item.id) {
                // Update existing widget only if data changed
                existing_widget.update_item_if_changed(item);
            } else {
                // Create new widget
                let widget = ModuleItemWidget::new();
                widget.set_item(item.clone());

                // Connect signals
                let window = self.clone();
                widget.connect_activate_item(move |widget| {
                    window.activate_item(widget);
                });

                let window = self.clone();
                widget.connect_context_menu_item(move |widget| {
                    window.show_context_menu(widget);
                });

                let window = self.clone();
                widget.connect_scroll_up(move |widget| {
                    window.invoke_item_action(widget, "volume_up");
                });

                let window = self.clone();
                widget.connect_scroll_down(move |widget| {
                    window.invoke_item_action(widget, "volume_down");
                });

                imp.items_box.append(&widget);
            }
        }

        // Update status label visibility
        if items.is_empty() {
            imp.status_label.set_text("No items");
            imp.status_label.set_visible(true);
        } else {
            imp.status_label.set_visible(false);
        }

        // Only grab focus on initial load, or if the focused item was removed
        let focused_item_removed = focused_item_id
            .as_ref()
            .map(|id| to_remove.contains(id))
            .unwrap_or(false);

        if was_empty || focused_item_removed {
            if let Some(first) = imp.items_box.first_child() {
                first.grab_focus();
            }
        }
    }

    /// Get the ID of the currently focused item, if any
    fn get_focused_item_id(&self) -> Option<String> {
        let imp = self.imp();
        let mut child = imp.items_box.first_child();
        while let Some(widget) = child {
            if widget.has_focus() || widget.is_focus() {
                if let Some(item_widget) = widget.downcast_ref::<ModuleItemWidget>() {
                    return item_widget.item_id();
                }
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Find a widget by item ID
    fn find_item_widget(&self, item_id: &str) -> Option<ModuleItemWidget> {
        let imp = self.imp();
        let mut child = imp.items_box.first_child();
        while let Some(widget) = child {
            if let Some(item_widget) = widget.downcast_ref::<ModuleItemWidget>() {
                if item_widget.item_id().as_deref() == Some(item_id) {
                    return Some(item_widget.clone());
                }
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Navigate between items using left/right arrows
    fn navigate_items(&self, direction: i32) {
        let imp = self.imp();

        // Find currently focused item index
        let mut current_index: Option<i32> = None;
        let mut count = 0i32;
        let mut child = imp.items_box.first_child();
        while let Some(widget) = child {
            if widget.has_focus() || widget.is_focus() {
                current_index = Some(count);
                break;
            }
            count += 1;
            child = widget.next_sibling();
        }

        // Calculate new index
        let total = self.item_count();
        if total == 0 {
            return;
        }

        let new_index = match current_index {
            Some(idx) => {
                let new = idx + direction;
                if new < 0 {
                    total - 1 // Wrap to end
                } else if new >= total {
                    0 // Wrap to start
                } else {
                    new
                }
            }
            None => 0, // No focus, start at beginning
        };

        // Focus the new item
        if let Some(widget) = self.child_at_index(new_index) {
            widget.grab_focus();
        }
    }

    /// Get total number of items
    fn item_count(&self) -> i32 {
        let imp = self.imp();
        let mut count = 0i32;
        let mut child = imp.items_box.first_child();
        while child.is_some() {
            count += 1;
            child = child.unwrap().next_sibling();
        }
        count
    }

    /// Get child widget at index
    fn child_at_index(&self, index: i32) -> Option<gtk4::Widget> {
        let imp = self.imp();
        let mut current = 0i32;
        let mut child = imp.items_box.first_child();
        while let Some(widget) = child {
            if current == index {
                return Some(widget);
            }
            current += 1;
            child = widget.next_sibling();
        }
        None
    }

    /// Listen for item changes from the daemon
    fn listen_for_changes(&self, client: Arc<DaemonClient>) {
        let window = self.clone();

        glib::spawn_future_local(async move {
            loop {
                // Listen for both legacy and module signals
                match client.wait_for_items_changed().await {
                    Ok(()) => {
                        tracing::debug!("Items changed, refreshing");
                        window.refresh_items().await;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to receive item changes: {}", e);
                        // Wait a bit before retrying
                        glib::timeout_future(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });
    }

    /// Activate a module item (invoke default action)
    fn activate_item(&self, widget: &ModuleItemWidget) {
        let Some(item_id) = widget.item_id() else {
            return;
        };

        // Get the default action for this item
        let Some(action_id) = widget.default_action_id() else {
            tracing::warn!("No default action for item: {}", item_id);
            return;
        };

        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        // Get position hint (center of the widget)
        let (x, y) = self.get_widget_position(widget);

        glib::spawn_future_local(async move {
            if let Err(e) = client.invoke_action(&item_id, &action_id, x, y).await {
                tracing::error!("Failed to invoke action {} on {}: {}", action_id, item_id, e);
            }
        });
    }

    /// Invoke a specific action on a module item
    fn invoke_item_action(&self, widget: &ModuleItemWidget, action_id: &str) {
        let Some(item_id) = widget.item_id() else {
            return;
        };

        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        let (x, y) = self.get_widget_position(widget);
        let action_id = action_id.to_string();

        glib::spawn_future_local(async move {
            if let Err(e) = client.invoke_action(&item_id, &action_id, x, y).await {
                tracing::error!("Failed to invoke action {} on {}: {}", action_id, item_id, e);
            }
        });
    }

    /// Show context menu for a module item
    fn show_context_menu(&self, widget: &ModuleItemWidget) {
        let Some(item_id) = widget.item_id() else {
            return;
        };

        // Check if item has a context menu action
        if !widget.has_context_menu() {
            tracing::debug!("Item {} has no context menu", item_id);
            return;
        }

        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        // Get position hint (center of the widget)
        let (x, y) = self.get_widget_position(widget);

        // Clone widget for async block
        let widget_clone = widget.clone();
        let item_id_clone = item_id.clone();
        let window = self.clone();

        glib::spawn_future_local(async move {
            // Try to get menu items via DBusMenu
            match client.get_item_menu(&item_id).await {
                Ok(items) if !items.is_empty() => {
                    // Show our custom popover menu
                    let popover = MenuPopover::new();
                    popover.set_parent(&widget_clone);
                    popover.set_client(client.clone());
                    popover.set_item_id(&item_id_clone);
                    popover.set_menu_items(&items);

                    // Track popover open/close to prevent window closing while menu is open
                    let imp = window.imp();
                    imp.open_popover_count.set(imp.open_popover_count.get() + 1);
                    popover.connect_closed(glib::clone!(
                        #[weak]
                        window,
                        move |popover| {
                            let imp = window.imp();

                            // Set suppression FIRST, before any operations that might trigger
                            // is-active changes (like decrementing popover count or grabbing focus)
                            let suppress_until = std::time::Instant::now() + std::time::Duration::from_millis(500);
                            imp.suppress_close_until.set(Some(suppress_until));

                            imp.open_popover_count.set(imp.open_popover_count.get().saturating_sub(1));

                            // Restore focus to the parent widget (the module item)
                            if let Some(parent) = popover.parent() {
                                parent.grab_focus();
                            }

                            // Schedule a check after suppression period - if focus didn't return, close
                            glib::timeout_add_local_once(std::time::Duration::from_millis(600), glib::clone!(
                                #[weak]
                                window,
                                move || {
                                    let imp = window.imp();
                                    if let Some(until) = imp.suppress_close_until.get() {
                                        if until == suppress_until && !window.is_active() && imp.open_popover_count.get() == 0 {
                                            window.close();
                                        }
                                    }
                                }
                            ));
                        }
                    ));

                    popover.popup();
                }
                Ok(_) => {
                    // Empty menu - try legacy SNI context_menu method
                    if let Err(e) = client.invoke_action(&item_id, "context_menu", x, y).await {
                        tracing::warn!("SNI context_menu failed for {}: {}", item_id, e);
                    }
                }
                Err(e) => {
                    // DBusMenu fetch failed - try legacy SNI context_menu method
                    tracing::debug!("DBusMenu failed for {}: {}, trying SNI fallback", item_id, e);
                    if let Err(e) = client.invoke_action(&item_id, "context_menu", x, y).await {
                        tracing::warn!("SNI context_menu also failed for {}: {}", item_id, e);
                    }
                }
            }
        });
    }

    /// Get the screen position of a widget (for menu positioning hints)
    fn get_widget_position(&self, widget: &ModuleItemWidget) -> (i32, i32) {
        // Try to get the position relative to the surface
        if let Some(native) = widget.native() {
            if let Some(_surface) = native.surface() {
                if let Some(point) =
                    widget.compute_point(&native, &gtk4::graphene::Point::new(0.0, 0.0))
                {
                    // This gives us position within the window
                    // For SNI items, they typically want screen coordinates
                    // We'll return the window-relative position and let the SNI item handle it
                    return (point.x() as i32, point.y() as i32);
                }
            }
        }
        (0, 0)
    }

    /// Show an error message
    fn show_error(&self, message: &str) {
        let imp = self.imp();
        imp.status_label.set_text(message);
        imp.status_label.set_visible(true);

        // Clear the items box
        while let Some(child) = imp.items_box.first_child() {
            imp.items_box.remove(&child);
        }
    }
}
