# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Build debug
cargo build

# Build release
cargo build --release

# Check compilation without building
cargo check

# Check only daemon (doesn't require GTK4)
cargo check -p waytray-daemon

# Run tests
cargo test
```

## System Dependencies

Requires GTK4 development libraries:
```bash
# Debian/Ubuntu
sudo apt install libgtk-4-dev
```

## Architecture

WayTray is a compositor-agnostic Linux system tray with a daemon + client architecture designed for accessibility.

### Daemon (`waytray-daemon`)

The daemon implements the [StatusNotifierItem (SNI)](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) protocol:

- **watcher.rs**: Fallback StatusNotifierWatcher implementation. Checks if an external watcher exists (e.g., from KDE/GNOME); if not, provides its own.
- **host.rs**: StatusNotifierHost that receives tray items via D-Bus. Subscribes to item signals (NewIcon, NewTitle, NewStatus, NewToolTip) for real-time updates.
- **cache.rs**: Thread-safe item cache with broadcast channel for change notifications.
- **dbus_service.rs**: Exposes `org.waytray.Daemon` interface for clients to query items and invoke actions (Activate, ContextMenu, etc.).

Shared types (`TrayItem`, `ItemStatus`, `ItemCategory`) are defined in `lib.rs` and used by both daemon and client.

### Client (`waytray-client`)

GTK4 application providing an accessible tray window:

- **daemon_proxy.rs**: zbus proxy for `org.waytray.Daemon` interface.
- **tray_item.rs**: GObject subclass of `ListBoxRow` with keyboard handling (Enter→Activate, Shift+F10/Menu→ContextMenu).
- **window.rs**: Main window with ListBox, handles D-Bus communication and item refresh.

### D-Bus Interfaces

- `org.kde.StatusNotifierWatcher` - Standard SNI watcher (fallback provided)
- `org.kde.StatusNotifierHost-{pid}` - Host registration
- `org.kde.StatusNotifierItem` - Individual tray items from applications
- `org.waytray.Daemon` - Custom interface for client-daemon IPC

### Key Implementation Details

**Service string parsing** (host.rs `parse_service_string`): SNI items register with various formats:
- Unique bus names: `:1.90/StatusNotifierItem`
- Well-known names: `org.kde.StatusNotifierItem-1234-1`
- Ayatana-style: `:1.75/org/ayatana/NotificationItem/app_name`

**Icon handling**: Prefer `icon_name` (theme lookup) over `icon_pixmap` (ARGB32 binary data requiring conversion to RGBA for GTK).

**Accessibility**: Items use `gtk4::AccessibleRole::Button` with proper labels for screen reader support.
