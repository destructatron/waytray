//! WayTray Client
//!
//! GTK4 client for displaying items from the WayTray daemon in an accessible window.
//! Uses a horizontal FlowBox for left/right arrow navigation like KDE's system tray.

mod app;
mod daemon_proxy;
mod menu_popover;
mod module_item;
mod window;

use gtk4::glib;
use tracing_subscriber::EnvFilter;

fn main() -> glib::ExitCode {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting WayTray client");

    // Run the GTK application
    let app = app::WayTrayApp::new();
    app.run()
}
