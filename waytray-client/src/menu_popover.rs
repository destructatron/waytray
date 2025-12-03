//! Menu popover for displaying DBusMenu items
//!
//! This module provides an accessible GTK4 Popover for displaying context menus
//! fetched via the DBusMenu protocol.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib};
use std::cell::RefCell;
use std::sync::Arc;

use waytray_daemon::dbus_service::MenuItemDto;

use crate::daemon_proxy::DaemonClient;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct MenuPopover {
        pub content_box: gtk4::Box,
        pub item_id: RefCell<String>,
        pub client: RefCell<Option<Arc<DaemonClient>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MenuPopover {
        const NAME: &'static str = "WayTrayMenuPopover";
        type Type = super::MenuPopover;
        type ParentType = gtk4::Popover;

        fn new() -> Self {
            Self {
                content_box: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                item_id: RefCell::new(String::new()),
                client: RefCell::new(None),
            }
        }
    }

    impl ObjectImpl for MenuPopover {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![Signal::builder("menu-item-activated")
                    .param_types([i32::static_type()])
                    .build()]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            // Configure the content box
            self.content_box.set_margin_top(4);
            self.content_box.set_margin_bottom(4);
            self.content_box.set_margin_start(4);
            self.content_box.set_margin_end(4);

            // Set accessible role for the menu
            self.content_box.set_accessible_role(gtk4::AccessibleRole::Menu);

            obj.set_child(Some(&self.content_box));

            // Set up keyboard navigation
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.connect_key_pressed(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, keyval, _keycode, _state| obj.handle_key_press(keyval)
            ));
            obj.add_controller(key_controller);
        }
    }

    impl WidgetImpl for MenuPopover {}
    impl PopoverImpl for MenuPopover {}
}

glib::wrapper! {
    pub struct MenuPopover(ObjectSubclass<imp::MenuPopover>)
        @extends gtk4::Popover, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Native, gtk4::ShortcutManager;
}

impl MenuPopover {
    /// Create a new menu popover
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the daemon client for menu item activation
    pub fn set_client(&self, client: Arc<DaemonClient>) {
        *self.imp().client.borrow_mut() = Some(client);
    }

    /// Set the tray item ID this menu belongs to
    pub fn set_item_id(&self, item_id: &str) {
        *self.imp().item_id.borrow_mut() = item_id.to_string();
    }

    /// Populate the menu with items (flat list with parent_id relationships)
    pub fn set_menu_items(&self, items: &[MenuItemDto]) {
        let imp = self.imp();

        // Clear existing children
        while let Some(child) = imp.content_box.first_child() {
            imp.content_box.remove(&child);
        }

        // Only show top-level items (parent_id == 0)
        // Submenus could be expanded inline or as nested popovers
        let top_level: Vec<_> = items.iter().filter(|i| i.parent_id == 0).collect();

        if top_level.is_empty() {
            let label = gtk4::Label::new(Some("No menu items"));
            label.set_margin_top(8);
            label.set_margin_bottom(8);
            label.set_margin_start(12);
            label.set_margin_end(12);
            imp.content_box.append(&label);
            return;
        }

        for item in top_level {
            let widget = self.create_menu_item_widget(item, items);
            imp.content_box.append(&widget);
        }

        // Focus the first item when menu is shown
        if let Some(first) = imp.content_box.first_child() {
            first.grab_focus();
        }
    }

    /// Create a widget for a single menu item
    fn create_menu_item_widget(&self, item: &MenuItemDto, all_items: &[MenuItemDto]) -> gtk4::Widget {
        if item.item_type == "separator" {
            let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            separator.set_margin_top(4);
            separator.set_margin_bottom(4);
            return separator.upcast();
        }

        let button = gtk4::Button::new();
        button.set_has_frame(false);

        // Create content box for the button
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        content.set_margin_start(8);
        content.set_margin_end(8);
        content.set_margin_top(4);
        content.set_margin_bottom(4);

        // Add toggle indicator if needed
        if !item.toggle_type.is_empty() {
            let indicator = if item.toggle_state == 1 {
                if item.toggle_type == "radio" {
                    gtk4::Image::from_icon_name("emblem-ok-symbolic")
                } else {
                    gtk4::Image::from_icon_name("emblem-ok-symbolic")
                }
            } else {
                // Placeholder for unchecked state
                gtk4::Image::new()
            };
            indicator.set_pixel_size(16);
            content.append(&indicator);
        }

        // Add icon if present
        if !item.icon_name.is_empty() {
            let icon = gtk4::Image::from_icon_name(&item.icon_name);
            icon.set_pixel_size(16);
            content.append(&icon);
        }

        // Add label
        let label = gtk4::Label::new(Some(&item.label));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        content.append(&label);

        // Add submenu indicator if has children
        if item.has_submenu {
            let arrow = gtk4::Image::from_icon_name("go-next-symbolic");
            arrow.set_pixel_size(16);
            content.append(&arrow);
        }

        button.set_child(Some(&content));

        // Set accessible properties
        button.set_accessible_role(gtk4::AccessibleRole::MenuItem);
        button.update_property(&[gtk4::accessible::Property::Label(&item.label)]);

        // Handle enabled state
        button.set_sensitive(item.enabled);

        // Store menu item ID and handle click
        let menu_item_id = item.id;
        let has_submenu = item.has_submenu;
        let popover = self.clone();

        // Get submenu items for this item
        let submenu_items: Vec<MenuItemDto> = if has_submenu {
            all_items
                .iter()
                .filter(|i| i.parent_id == menu_item_id)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        button.connect_clicked(move |btn| {
            if has_submenu && !submenu_items.is_empty() {
                // Show submenu as a nested popover
                let submenu_popover = MenuPopover::new();
                submenu_popover.set_parent(btn);
                submenu_popover.set_client(
                    popover
                        .imp()
                        .client
                        .borrow()
                        .clone()
                        .expect("Client not set"),
                );
                submenu_popover.set_item_id(&popover.imp().item_id.borrow());

                // Build flat list for submenu (items where parent_id matches)
                let mut submenu_flat = submenu_items.clone();
                // Include nested items too
                submenu_flat.iter_mut().for_each(|i| i.parent_id = 0);
                submenu_popover.set_menu_items(&submenu_flat);

                // Forward activation signal
                let parent_popover = popover.clone();
                submenu_popover.connect_local("menu-item-activated", false, move |values| {
                    let id = values[1].get::<i32>().unwrap();
                    parent_popover.emit_by_name::<()>("menu-item-activated", &[&id]);
                    None
                });

                submenu_popover.popup();
            } else {
                // Activate this menu item
                popover.activate_menu_item(menu_item_id);
            }
        });

        button.upcast()
    }

    /// Activate a menu item
    fn activate_menu_item(&self, menu_item_id: i32) {
        let imp = self.imp();
        let item_id = imp.item_id.borrow().clone();
        let client = imp.client.borrow().clone();

        // Emit signal for the caller
        self.emit_by_name::<()>("menu-item-activated", &[&menu_item_id]);

        // Close the popover
        self.popdown();

        // Actually activate via D-Bus
        if let Some(client) = client {
            glib::spawn_future_local(async move {
                if let Err(e) = client.activate_menu_item(&item_id, menu_item_id).await {
                    tracing::error!("Failed to activate menu item {}: {}", menu_item_id, e);
                }
            });
        }
    }

    /// Handle keyboard navigation
    fn handle_key_press(&self, keyval: gdk::Key) -> glib::Propagation {
        match keyval {
            gdk::Key::Escape => {
                self.popdown();
                glib::Propagation::Stop
            }
            gdk::Key::Up => {
                self.navigate_items(-1);
                glib::Propagation::Stop
            }
            gdk::Key::Down => {
                self.navigate_items(1);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                // Activate the focused button
                if let Some(focused) = self.focus_child() {
                    if let Some(button) = focused.downcast_ref::<gtk4::Button>() {
                        button.emit_clicked();
                    }
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// Navigate between menu items
    fn navigate_items(&self, direction: i32) {
        let imp = self.imp();

        // Find currently focused item
        let mut current_index: Option<i32> = None;
        let mut focusable_children = Vec::new();

        let mut child = imp.content_box.first_child();
        while let Some(widget) = child {
            // Only count buttons (not separators)
            if widget.downcast_ref::<gtk4::Button>().is_some() {
                if widget.has_focus() || widget.is_focus() {
                    current_index = Some(focusable_children.len() as i32);
                }
                focusable_children.push(widget.clone());
            }
            child = widget.next_sibling();
        }

        let total = focusable_children.len() as i32;
        if total == 0 {
            return;
        }

        let new_index = match current_index {
            Some(idx) => {
                let new = idx + direction;
                if new < 0 {
                    total - 1
                } else if new >= total {
                    0
                } else {
                    new
                }
            }
            None => 0,
        };

        if let Some(widget) = focusable_children.get(new_index as usize) {
            widget.grab_focus();
        }
    }

    /// Connect to the menu-item-activated signal
    pub fn connect_menu_item_activated<F: Fn(&Self, i32) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("menu-item-activated", false, move |values| {
            let obj = values[0].get::<MenuPopover>().unwrap();
            let id = values[1].get::<i32>().unwrap();
            f(&obj, id);
            None
        })
    }
}

impl Default for MenuPopover {
    fn default() -> Self {
        Self::new()
    }
}
