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

Requires GTK4, GStreamer, and GTK4 Layer Shell development libraries:
```bash
# Debian/Ubuntu
sudo apt install libgtk-4-dev libgstreamer1.0-dev libgtk4-layer-shell-dev

# Gentoo
emerge gui-libs/gtk4-layer-shell  # May need keyword unmasking
```

For the pipewire module, ensure `pactl` is available (from `pulseaudio-utils` or `pipewire-pulse`).

**Note**: The client uses GTK Layer Shell for Wayland overlay support (appearing above fullscreen windows). This is automatically detected at runtime - on X11 or when layer shell is unavailable, it falls back to a regular window.

## Architecture

WayTray is a compositor-agnostic Linux system tray with a daemon + client architecture designed for accessibility.

### Daemon (`waytray-daemon`)

The daemon uses a modular architecture configured via TOML:

#### Core Files
- **main.rs**: Entry point, loads config, initializes modules and D-Bus services
- **config.rs**: TOML configuration from `~/.config/waytray/config.toml` (auto-created with defaults)
- **config_watcher.rs**: File watcher for config hot reload (uses `notify` crate)
- **dbus_service.rs**: Exposes `org.waytray.Daemon` interface for clients
- **dbusmenu.rs**: DBusMenu protocol (`com.canonical.dbusmenu`) for fetching/activating app menus
- **notifications.rs**: Desktop notifications via freedesktop notification spec (notify-rust)
- **watcher.rs**: Fallback StatusNotifierWatcher if none exists (e.g., from KDE/GNOME)
- **host.rs**: StatusNotifierHost that receives tray items via D-Bus

#### Module System (`modules/`)
- **mod.rs**: `Module` trait, `ModuleRegistry`, `ModuleItem`, `ModuleContext`, event broadcasting
- **tray.rs**: System tray items via StatusNotifierItem protocol
- **battery.rs**: Battery status via UPower D-Bus, notifications with optional GStreamer sounds
- **clock.rs**: Time display with configurable strftime format
- **system.rs**: CPU/memory/temperature from `/proc/stat`, `/proc/meminfo`, `/sys/class/thermal`
- **network.rs**: Network status and speeds from `/sys/class/net` and `/proc/net/route`
- **pipewire.rs**: Audio output and microphone volume control via PulseAudio/PipeWire (pactl)
- **power_profiles.rs**: Power profile switching via power-profiles-daemon D-Bus
- **weather.rs**: Weather via wttr.in API (no API key required)
- **gpu.rs**: GPU usage/temperature via nvidia-smi (NVIDIA) or sysfs (AMD)

#### Module Trait
```rust
#[async_trait]
pub trait Module: Send + Sync {
    fn name(&self) -> &str;
    fn enabled(&self) -> bool;
    async fn start(&self, ctx: Arc<ModuleContext>);
    async fn stop(&self);
    async fn invoke_action(&self, item_id: &str, action_id: &str, x: i32, y: i32);
    async fn reload_config(&self, config: &Config) -> bool; // Hot reload support
}
```

Modules emit `ModuleEvent::ItemsUpdated` via `ModuleContext` to update the registry.
Modules store config in `RwLock` to support hot reload when the config file changes.

### Client (`waytray-client`)

GTK4 application providing an accessible panel window:

- **main.rs**: Entry point, creates application and window
- **daemon_proxy.rs**: D-Bus client for `org.waytray.Daemon` interface
- **module_item.rs**: `ModuleItemWidget` - GObject Box subclass with keyboard handling (Enter→Activate, Shift+F10/Menu→ContextMenu)
- **menu_popover.rs**: GTK4 Popover for rendering DBusMenu context menus with accessibility support
- **window.rs**: Main window with horizontal `gtk4::Box`, left/right arrow navigation, incremental updates to preserve accessibility state

#### Accessibility
- Horizontal Box layout (FlowBox caused Orca screen reader issues)
- Left/Right arrows navigate between items (with wrapping)
- Enter/Space activates, Shift+F10/Menu opens context menu
- Incremental updates avoid re-announcing unchanged items to screen readers
- Items use `gtk4::AccessibleRole::Button` with proper labels

### D-Bus Interfaces

- `org.kde.StatusNotifierWatcher` - Standard SNI watcher (fallback provided)
- `org.kde.StatusNotifierHost-{pid}` - Host registration
- `org.kde.StatusNotifierItem` - Individual tray items from applications
- `com.canonical.dbusmenu` - Application menus (used when SNI `ContextMenu` method unavailable)
- `org.waytray.Daemon` - Custom interface for client-daemon IPC

### Key Implementation Details

**Service string parsing** (host.rs `parse_service_string`): SNI items register with various formats:
- Unique bus names: `:1.90/StatusNotifierItem`
- Well-known names: `org.kde.StatusNotifierItem-1234-1`
- Ayatana-style: `:1.75/org/ayatana/NotificationItem/app_name`

**Icon handling**: Prefer `icon_name` (theme lookup) over `icon_pixmap` (ARGB32 binary data requiring conversion to RGBA for GTK).

**Weather API**: Uses wttr.in with `curl` user agent (wttr.in blocks non-curl user agents). JSON endpoint includes both Celsius and Fahrenheit.

**DBusMenu support**: Some apps (e.g., Steam) don't implement the SNI `ContextMenu(x, y)` method but expose menus via `com.canonical.dbusmenu`. The daemon fetches menus using `GetLayout` and activates items via `Event("clicked")`. Menu items are flattened (parent_id relationships) for D-Bus transport, then rendered as an accessible GTK4 Popover in the client. Falls back to SNI `ContextMenu` if DBusMenu fails.

## Version Management

When updating the application version, the following files need to be changed:

1. **Cargo.toml** (workspace root): `version = "X.Y.Z"` - The daemon and client inherit this via `version.workspace = true`
2. **packaging/rpm/waytray.spec**: `Version:` field and add a new `%changelog` entry

## Configuration

Config file: `~/.config/waytray/config.toml` (created automatically with defaults)

```toml
[modules]
order = ["tray", "pipewire", "power_profiles", "battery", "system", "gpu", "network", "weather", "clock"]

[modules.tray]
enabled = true

[modules.battery]
enabled = true
low_threshold = 20
critical_threshold = 10
notify_full_charge = false
# low_sound = "~/.config/waytray/sounds/low.wav"
# critical_sound = "~/.config/waytray/sounds/critical.wav"
# full_sound = "~/.config/waytray/sounds/full.wav"

[modules.clock]
enabled = true
format = "%H:%M"
date_format = "%A, %B %d, %Y"

[modules.system]
enabled = true
show_cpu = true
show_memory = true
show_temperature = false
show_top_cpu_process = false    # Show top CPU process in tooltip
show_top_memory_process = false # Show top memory process in tooltip
interval_seconds = 5

[modules.network]
enabled = true
interface = ""          # Empty = auto-detect
show_ip = false
show_speed = true
interval_seconds = 2

[modules.weather]
enabled = true
location = ""           # Empty = auto-detect from IP
units = "celsius"       # or "fahrenheit"
interval_seconds = 1800

[modules.pipewire]
enabled = true
show_volume = true      # Show volume percentage in label
max_volume = 100        # Maximum volume (100 = normal, 150 = allow boost)
scroll_step = 5         # Volume change per scroll step
show_microphone = true  # Show microphone control item
show_mic_volume = true  # Show mic volume percentage in label
mic_max_volume = 100    # Maximum mic volume (100 = normal, 150 = allow boost)
mic_scroll_step = 5     # Mic volume change per scroll step

[modules.power_profiles]
enabled = true          # Requires power-profiles-daemon

[modules.gpu]
enabled = true
show_temperature = false    # Show GPU temperature
show_top_process = false    # Show top GPU memory process (NVIDIA only)
interval_seconds = 5

[notifications]
enabled = true
timeout_ms = 5000
```
