//! Main window for the WayTray client

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, gio, glib};
use std::cell::RefCell;
use std::sync::Arc;

use crate::daemon_proxy::DaemonClient;
use crate::tray_item::TrayItemRow;
use waytray_daemon::TrayItem;

mod imp {
    use super::*;
    use gtk4::subclass::application_window::ApplicationWindowImpl;
    use gtk4::subclass::widget::WidgetImpl;
    use gtk4::subclass::window::WindowImpl;

    pub struct WayTrayWindow {
        pub list_box: gtk4::ListBox,
        pub scrolled_window: gtk4::ScrolledWindow,
        pub status_label: gtk4::Label,
        pub main_box: gtk4::Box,
        pub client: RefCell<Option<Arc<DaemonClient>>>,
    }

    impl Default for WayTrayWindow {
        fn default() -> Self {
            Self {
                list_box: gtk4::ListBox::new(),
                scrolled_window: gtk4::ScrolledWindow::new(),
                status_label: gtk4::Label::new(Some("Connecting to daemon...")),
                main_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                client: RefCell::new(None),
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
            obj.set_default_size(300, 400);
            obj.set_resizable(true);

            // Configure the list box
            self.list_box.set_selection_mode(gtk4::SelectionMode::Single);
            self.list_box.set_activate_on_single_click(false);
            self.list_box.add_css_class("boxed-list");

            // Set accessible properties for the list
            self.list_box
                .set_accessible_role(gtk4::AccessibleRole::List);

            // Handle row activation (double-click or Enter)
            self.list_box.connect_row_activated(glib::clone!(
                #[weak]
                obj,
                move |_, row| {
                    if let Some(tray_row) = row.downcast_ref::<TrayItemRow>() {
                        obj.activate_item(tray_row);
                    }
                }
            ));

            // Configure scrolled window
            self.scrolled_window.set_child(Some(&self.list_box));
            self.scrolled_window.set_vexpand(true);
            self.scrolled_window.set_hexpand(true);
            self.scrolled_window
                .set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

            // Configure status label
            self.status_label.set_margin_top(12);
            self.status_label.set_margin_bottom(12);
            self.status_label.set_margin_start(12);
            self.status_label.set_margin_end(12);

            // Build layout
            self.main_box.append(&self.status_label);
            self.main_box.append(&self.scrolled_window);

            obj.set_child(Some(&self.main_box));

            // Set up keyboard handling for the window
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.connect_key_pressed(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, keyval, _keycode, _state| {
                    match keyval {
                        gdk::Key::Escape => {
                            obj.close();
                            glib::Propagation::Stop
                        }
                        _ => glib::Propagation::Proceed,
                    }
                }
            ));
            obj.add_controller(key_controller);
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

    /// Refresh the list of tray items
    async fn refresh_items(&self) {
        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        match client.get_items().await {
            Ok(items) => {
                self.update_items(&items);
            }
            Err(e) => {
                tracing::error!("Failed to get items: {}", e);
                self.show_error(&format!("Failed to get items: {}", e));
            }
        }
    }

    /// Update the displayed items
    fn update_items(&self, items: &[TrayItem]) {
        let imp = self.imp();

        // Clear existing items
        while let Some(child) = imp.list_box.first_child() {
            imp.list_box.remove(&child);
        }

        if items.is_empty() {
            imp.status_label.set_text("No tray items");
            imp.status_label.set_visible(true);
        } else {
            imp.status_label.set_visible(false);

            for item in items {
                let row = TrayItemRow::new();
                row.set_item(item.clone());

                // Connect signals
                let window = self.clone();
                row.connect_activate_item(move |row| {
                    window.activate_item(row);
                });

                let window = self.clone();
                row.connect_context_menu_item(move |row| {
                    window.show_context_menu(row);
                });

                imp.list_box.append(&row);
            }

            // Focus the first item
            if let Some(first) = imp.list_box.row_at_index(0) {
                first.grab_focus();
            }
        }
    }

    /// Listen for item changes from the daemon
    fn listen_for_changes(&self, client: Arc<DaemonClient>) {
        let window = self.clone();

        glib::spawn_future_local(async move {
            loop {
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

    /// Activate a tray item (primary action)
    fn activate_item(&self, row: &TrayItemRow) {
        let Some(item_id) = row.item_id() else {
            return;
        };

        // If item is menu-only, show context menu instead
        if row.is_menu_only() {
            self.show_context_menu(row);
            return;
        }

        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        // Get position hint (center of the row)
        let (x, y) = self.get_row_position(row);

        glib::spawn_future_local(async move {
            if let Err(e) = client.activate(&item_id, x, y).await {
                tracing::error!("Failed to activate item {}: {}", item_id, e);
            }
        });
    }

    /// Show context menu for a tray item
    fn show_context_menu(&self, row: &TrayItemRow) {
        let Some(item_id) = row.item_id() else {
            return;
        };

        let client = self.imp().client.borrow().clone();
        let Some(client) = client else {
            return;
        };

        // Get position hint (center of the row)
        let (x, y) = self.get_row_position(row);

        glib::spawn_future_local(async move {
            if let Err(e) = client.context_menu(&item_id, x, y).await {
                tracing::error!("Failed to show context menu for item {}: {}", item_id, e);
            }
        });
    }

    /// Get the screen position of a row (for menu positioning hints)
    fn get_row_position(&self, row: &TrayItemRow) -> (i32, i32) {
        // Try to get the position relative to the surface
        if let Some(native) = row.native() {
            if let Some(_surface) = native.surface() {
                if let Some(point) =
                    row.compute_point(&native, &gtk4::graphene::Point::new(0.0, 0.0))
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

        // Clear the list
        while let Some(child) = imp.list_box.first_child() {
            imp.list_box.remove(&child);
        }
    }
}
