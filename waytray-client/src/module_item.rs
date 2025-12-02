//! Module item widget for displaying a single item from any module

use glib::subclass::Signal;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib};
use std::cell::RefCell;
use std::sync::OnceLock;

use waytray_daemon::ModuleItem;

mod imp {
    use super::*;
    use gtk4::subclass::flow_box_child::FlowBoxChildImpl;
    use gtk4::subclass::widget::WidgetImpl;

    #[derive(Default)]
    pub struct ModuleItemWidget {
        pub item_data: RefCell<Option<ModuleItem>>,
        pub icon: gtk4::Image,
        pub label: gtk4::Label,
        pub hbox: gtk4::Box,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ModuleItemWidget {
        const NAME: &'static str = "WayTrayModuleItem";
        type Type = super::ModuleItemWidget;
        type ParentType = gtk4::FlowBoxChild;

        fn new() -> Self {
            Self {
                item_data: RefCell::new(None),
                icon: gtk4::Image::new(),
                label: gtk4::Label::new(None),
                hbox: gtk4::Box::new(gtk4::Orientation::Horizontal, 8),
            }
        }
    }

    impl ObjectImpl for ModuleItemWidget {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("activate-item").build(),
                    Signal::builder("context-menu-item").build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            // Configure the icon
            self.icon.set_pixel_size(24);

            // Configure the label
            self.label.set_xalign(0.0);

            // Build the layout - more compact for horizontal flow
            self.hbox.set_margin_start(8);
            self.hbox.set_margin_end(8);
            self.hbox.set_margin_top(6);
            self.hbox.set_margin_bottom(6);
            self.hbox.append(&self.icon);
            self.hbox.append(&self.label);

            obj.set_child(Some(&self.hbox));

            // Make focusable
            obj.set_focusable(true);

            // Set accessible role
            obj.set_accessible_role(gtk4::AccessibleRole::Button);

            // Set up key event controller
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.connect_key_pressed(
                glib::clone!(
                    #[weak]
                    obj,
                    #[upgrade_or]
                    glib::Propagation::Proceed,
                    move |_, keyval, _keycode, state| obj.handle_key_press(keyval, state)
                ),
            );
            obj.add_controller(key_controller);
        }
    }

    impl WidgetImpl for ModuleItemWidget {}
    impl FlowBoxChildImpl for ModuleItemWidget {}
}

glib::wrapper! {
    pub struct ModuleItemWidget(ObjectSubclass<imp::ModuleItemWidget>)
        @extends gtk4::FlowBoxChild, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ModuleItemWidget {
    /// Create a new module item widget
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the module item data
    pub fn set_item(&self, item: ModuleItem) {
        let imp = self.imp();

        // Set the label
        imp.label.set_text(&item.label);

        // Set the icon
        self.update_icon(&item);

        // Set accessible properties
        self.update_property(&[gtk4::accessible::Property::Label(&item.label)]);

        if let Some(tooltip) = &item.tooltip {
            self.set_tooltip_text(Some(tooltip));
            self.update_property(&[gtk4::accessible::Property::Description(tooltip)]);
        }

        // Store the item data
        *imp.item_data.borrow_mut() = Some(item);
    }

    /// Update the icon from item data
    fn update_icon(&self, item: &ModuleItem) {
        let imp = self.imp();

        // Prefer icon name (from theme)
        if let Some(icon_name) = &item.icon_name {
            if !icon_name.is_empty() {
                imp.icon.set_icon_name(Some(icon_name));
                return;
            }
        }

        // Fall back to pixmap data
        if let Some(pixmap_data) = &item.icon_pixmap {
            if !pixmap_data.is_empty() && item.icon_width > 0 && item.icon_height > 0 {
                if let Some(texture) =
                    self.create_texture_from_argb32(pixmap_data, item.icon_width, item.icon_height)
                {
                    imp.icon.set_paintable(Some(&texture));
                    return;
                }
            }
        }

        // Default fallback icon
        imp.icon.set_icon_name(Some("application-x-executable"));
    }

    /// Create a GDK texture from ARGB32 pixmap data
    fn create_texture_from_argb32(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Option<gdk::Texture> {
        // SNI pixmaps are in ARGB32 format (network byte order, i.e., big-endian)
        // We need to convert to RGBA for GDK
        let expected_size = (width * height * 4) as usize;
        if data.len() < expected_size {
            tracing::warn!(
                "Pixmap data too small: {} < {} ({}x{})",
                data.len(),
                expected_size,
                width,
                height
            );
            return None;
        }

        // Convert ARGB (big-endian) to RGBA
        let mut rgba_data = Vec::with_capacity(expected_size);
        for chunk in data[..expected_size].chunks(4) {
            if chunk.len() == 4 {
                // ARGB -> RGBA
                let a = chunk[0];
                let r = chunk[1];
                let g = chunk[2];
                let b = chunk[3];
                rgba_data.extend_from_slice(&[r, g, b, a]);
            }
        }

        let bytes = glib::Bytes::from(&rgba_data);
        let texture = gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            (width * 4) as usize,
        );

        Some(texture.upcast())
    }

    /// Get the item ID
    pub fn item_id(&self) -> Option<String> {
        self.imp()
            .item_data
            .borrow()
            .as_ref()
            .map(|item| item.id.clone())
    }

    /// Get the default action ID for this item
    pub fn default_action_id(&self) -> Option<String> {
        self.imp()
            .item_data
            .borrow()
            .as_ref()
            .and_then(|item| {
                item.actions
                    .iter()
                    .find(|a| a.is_default)
                    .or_else(|| item.actions.first())
                    .map(|a| a.id.clone())
            })
    }

    /// Check if this item has a context menu action
    pub fn has_context_menu(&self) -> bool {
        self.imp()
            .item_data
            .borrow()
            .as_ref()
            .map(|item| {
                item.actions.iter().any(|a| a.id == "context_menu")
            })
            .unwrap_or(false)
    }

    /// Handle key press events
    fn handle_key_press(&self, keyval: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        match keyval {
            // Enter or Space: Activate the item (default action)
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                self.emit_by_name::<()>("activate-item", &[]);
                glib::Propagation::Stop
            }

            // Shift+F10 or Menu key: Show context menu
            gdk::Key::F10 if state.contains(gdk::ModifierType::SHIFT_MASK) => {
                self.emit_by_name::<()>("context-menu-item", &[]);
                glib::Propagation::Stop
            }
            gdk::Key::Menu => {
                self.emit_by_name::<()>("context-menu-item", &[]);
                glib::Propagation::Stop
            }

            _ => glib::Propagation::Proceed,
        }
    }

    /// Connect to the activate-item signal
    pub fn connect_activate_item<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("activate-item", false, move |values| {
            let obj = values[0].get::<ModuleItemWidget>().unwrap();
            f(&obj);
            None
        })
    }

    /// Connect to the context-menu-item signal
    pub fn connect_context_menu_item<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("context-menu-item", false, move |values| {
            let obj = values[0].get::<ModuleItemWidget>().unwrap();
            f(&obj);
            None
        })
    }
}

impl Default for ModuleItemWidget {
    fn default() -> Self {
        Self::new()
    }
}
