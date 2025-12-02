//! GTK4 Application setup

use gtk4::prelude::*;
use gtk4::{gio, glib};

use crate::window::WayTrayWindow;

/// The main GTK application
pub struct WayTrayApp {
    app: gtk4::Application,
}

impl WayTrayApp {
    pub fn new() -> Self {
        let app = gtk4::Application::builder()
            .application_id("org.waytray.Client")
            .flags(gio::ApplicationFlags::FLAGS_NONE)
            .build();

        app.connect_activate(|app| {
            // Check if we already have a window
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }

            // Create the main window
            let window = WayTrayWindow::new(app);
            window.present();
        });

        Self { app }
    }

    pub fn run(&self) -> glib::ExitCode {
        self.app.run()
    }
}

impl Default for WayTrayApp {
    fn default() -> Self {
        Self::new()
    }
}
