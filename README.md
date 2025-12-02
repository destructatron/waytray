# WayTray

An accessible, compositor-agnostic system tray for Linux.

WayTray implements the [StatusNotifierItem (SNI)](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) specification with a daemon + client architecture. Unlike traditional system trays that sit permanently in a panel, WayTray's client opens as a regular window on demand, making it fully accessible to screen readers like Orca.

## Features

- **Compositor agnostic**: Works on any Wayland compositor (or X11)
- **Accessible**: Full keyboard navigation and screen reader support
  - Left/Right arrows to navigate between items
  - Enter/Space to activate items
  - Shift+F10 or Menu key for context menus
  - Escape to close
- **Modular**: Built-in modules for system tray, battery, clock, and more
- **Configurable**: TOML configuration for modules, ordering, and notifications
- **Daemon + client architecture**: Daemon caches items; client displays them on demand
- **Real-time updates**: Items refresh automatically when status changes
- **Desktop notifications**: Battery warnings and other alerts via freedesktop notifications

## Dependencies

### Build Dependencies

**Debian/Ubuntu:**
```bash
sudo apt install build-essential pkg-config libgtk-4-dev
```

**Fedora:**
```bash
sudo dnf install gcc pkg-config gtk4-devel
```

**Arch:**
```bash
sudo pacman -S base-devel gtk4
```

### Runtime Dependencies

- GTK4
- A D-Bus session bus (standard on most Linux desktops)

## Building

```bash
git clone https://github.com/destructatron/waytray
cd waytray
cargo build --release
```

Binaries will be at:
- `target/release/waytray-daemon`
- `target/release/waytray`

## Usage

### Start the daemon

```bash
./target/release/waytray-daemon
```

The daemon registers as a StatusNotifierHost and begins caching tray items from applications (Discord, Spotify, nm-applet, etc.).

### Open the client

```bash
./target/release/waytray
```

A window appears showing all current tray items. Interact with them using keyboard or mouse, then close the window when done.

### Systemd user service (optional)

To start the daemon automatically at login:

```bash
mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/waytray.service << 'EOF'
[Unit]
Description=WayTray System Tray Daemon

[Service]
ExecStart=%h/.local/bin/waytray-daemon
Restart=on-failure

[Install]
WantedBy=default.target
EOF

# Copy binary to ~/.local/bin (or adjust the path above)
cp target/release/waytray-daemon ~/.local/bin/

# Enable and start
systemctl --user enable --now waytray
```

Then bind `waytray` to a keyboard shortcut in your compositor for quick access.

## Configuration

WayTray is configured via a TOML file at `~/.config/waytray/config.toml`. If the file doesn't exist, it is created with defaults.

**Hot Reload**: The daemon automatically watches the config file and reloads module settings when it changes. No restart required for most configuration changes.

### Example Configuration

```toml
[modules]
# Module display order (left to right)
# Modules not listed appear after these
order = ["tray", "battery", "system", "weather", "clock"]

[modules.tray]
enabled = true

[modules.battery]
enabled = true
low_threshold = 20          # Notify at this percentage
critical_threshold = 10     # Critical notification at this percentage
notify_full_charge = false  # Notify when fully charged

[modules.clock]
enabled = true
format = "%H:%M"                    # Time format (strftime)
date_format = "%A, %B %d, %Y"       # Tooltip date format

[modules.system]
enabled = true
show_cpu = true
show_memory = true
show_temperature = false
interval_seconds = 5

[modules.weather]
enabled = true
location = ""                 # Empty = auto-detect from IP
units = "celsius"             # or "fahrenheit"
interval_seconds = 1800       # 30 minutes

[notifications]
enabled = true
timeout_ms = 5000  # 0 = no timeout
```

### Modules

#### Tray (`[modules.tray]`)

Displays system tray items from applications (Discord, Spotify, nm-applet, etc.) using the StatusNotifierItem protocol.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the tray module |

#### Battery (`[modules.battery]`)

Displays battery status and sends notifications for low/critical/full states. Uses UPower via D-Bus.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the battery module |
| `low_threshold` | u8 | `20` | Battery percentage for low warning |
| `critical_threshold` | u8 | `10` | Battery percentage for critical warning |
| `notify_full_charge` | bool | `false` | Send notification when fully charged |

#### Clock (`[modules.clock]`)

Displays the current time with configurable format.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the clock module |
| `format` | string | `"%H:%M"` | Time format ([strftime](https://strftime.org/)) |
| `date_format` | string | `"%A, %B %d, %Y"` | Tooltip date format |

**Format examples:**
- `%H:%M` - 24-hour time (14:30)
- `%I:%M %p` - 12-hour with AM/PM (2:30 PM)
- `%H:%M:%S` - With seconds (14:30:45)

#### System (`[modules.system]`)

Displays CPU usage, memory usage, and CPU temperature. Reads from `/proc/stat`, `/proc/meminfo`, and `/sys/class/thermal`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the system module |
| `show_cpu` | bool | `true` | Show CPU usage percentage |
| `show_memory` | bool | `true` | Show memory usage percentage |
| `show_temperature` | bool | `false` | Show CPU temperature |
| `interval_seconds` | u64 | `5` | Update interval in seconds |

#### Weather (`[modules.weather]`)

Displays current weather conditions using [wttr.in](https://wttr.in) (no API key required). Shows temperature as the label with detailed conditions in the tooltip.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the weather module |
| `location` | string | `""` | City name (e.g., "London", "New York"). Empty = auto-detect from IP |
| `units` | string | `"celsius"` | Temperature units: "celsius" or "fahrenheit" |
| `interval_seconds` | u64 | `1800` | Update interval (default 30 minutes) |

**Display:**
- Label: Temperature (e.g., "15°C")
- Tooltip: Conditions, feels like, humidity, location
- Icon: Weather-appropriate theme icon

### Notifications (`[notifications]`)

Global notification settings.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable all notifications |
| `timeout_ms` | u32 | `5000` | Notification timeout (0 = never) |

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  waytray-daemon (always running)                        │
│  ├─ Module system                                       │
│  │   ├─ Tray module (SNI host for app tray items)       │
│  │   ├─ Battery module (UPower D-Bus)                   │
│  │   ├─ Clock module (time display)                     │
│  │   ├─ System module (CPU/memory from /proc)           │
│  │   └─ Weather module (wttr.in API)                    │
│  ├─ StatusNotifierWatcher (fallback if none exists)     │
│  ├─ Notification service (desktop notifications)        │
│  └─ org.waytray.Daemon interface for clients            │
└─────────────────────────────────────────────────────────┘
                            ↕ D-Bus
┌─────────────────────────────────────────────────────────┐
│  waytray (GTK4 client, invoked on demand)               │
│  ├─ Queries daemon for module items                     │
│  ├─ Displays accessible horizontal item list            │
│  └─ Forwards user actions to daemon                     │
└─────────────────────────────────────────────────────────┘
```

## License

MIT
