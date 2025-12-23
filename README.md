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

WayTray is written in Rust. Install Rust and the required development libraries for your distribution:

**Debian/Ubuntu:**
```bash
sudo apt install rustc cargo build-essential pkg-config libgtk-4-dev libgstreamer1.0-dev
```

**Fedora:**
```bash
sudo dnf install rust cargo gcc pkg-config gtk4-devel gstreamer1-devel
```

**Arch:**
```bash
sudo pacman -S rust base-devel gtk4 gstreamer
```

If the packaged Rust version doesn't work, install via [rustup](https://rustup.rs/) instead:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Runtime Dependencies

- GTK4
- A D-Bus session bus (standard on most Linux desktops)
- `pactl` for pipewire module (from `pulseaudio-utils` or `pipewire-pulse`)

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
order = ["tray", "pipewire", "power_profiles", "battery", "system", "gpu", "network", "weather", "clock", "scripts"]

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
show_top_cpu_process = false      # Show top CPU process in tooltip
show_top_memory_process = false   # Show top memory process in tooltip
interval_seconds = 5

[modules.weather]
enabled = true
location = ""                 # Empty = auto-detect from IP
units = "celsius"             # or "fahrenheit"
interval_seconds = 1800       # 30 minutes

[modules.network]
enabled = true
interface = ""                # Empty = auto-detect default route
show_ip = false
show_speed = true
interval_seconds = 2

[modules.pipewire]
enabled = true
show_volume = true            # Show volume % in label
max_volume = 100              # Cap volume (100 = normal, 150 = boost)
scroll_step = 5               # Volume change per action
show_microphone = true        # Show microphone control item
show_mic_volume = true        # Show mic volume % in label
mic_max_volume = 100          # Cap mic volume (100 = normal, 150 = boost)
mic_scroll_step = 5           # Mic volume change per action

[modules.power_profiles]
enabled = true                # Requires power-profiles-daemon

[modules.gpu]
enabled = true
show_temperature = false      # Show GPU temperature in tooltip
show_top_process = false      # Show top GPU memory process (NVIDIA only)
interval_seconds = 5

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

Displays battery status and sends notifications for low/critical/full states. Uses UPower via D-Bus. Optionally plays sounds using GStreamer.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the battery module |
| `low_threshold` | u8 | `20` | Battery percentage for low warning |
| `critical_threshold` | u8 | `10` | Battery percentage for critical warning |
| `notify_full_charge` | bool | `false` | Send notification when fully charged |
| `low_sound` | string | `null` | Sound file to play on low battery (optional) |
| `critical_sound` | string | `null` | Sound file to play on critical battery (optional) |
| `full_sound` | string | `null` | Sound file to play when fully charged (optional) |

**Sound files:** Paths can use `~` for home directory. Supports any format GStreamer can play (WAV, OGG, MP3, etc.).

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
| `show_top_cpu_process` | bool | `false` | Show top CPU process in CPU tooltip |
| `show_top_memory_process` | bool | `false` | Show top memory process in memory tooltip |
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

#### Network (`[modules.network]`)

Displays network connection status and transfer speeds. Reads from `/sys/class/net` and `/proc/net/route`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the network module |
| `interface` | string | `""` | Network interface to monitor. Empty = auto-detect default route interface |
| `show_ip` | bool | `false` | Show IP address in tooltip |
| `show_speed` | bool | `true` | Show upload/download speeds |
| `interval_seconds` | u64 | `2` | Update interval in seconds |

**Display:**
- Label: Upload/download speeds (e.g., "↓1.2MB/s ↑50KB/s")
- Tooltip: Interface name, IP address, speeds
- Icon: `network-wireless`, `network-wired`, or `network-offline` based on interface type and status

#### Pipewire (`[modules.pipewire]`)

Displays audio output volume and microphone input controls. Uses `pactl` to communicate with PulseAudio or PipeWire (via pipewire-pulse).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the pipewire module |
| `show_volume` | bool | `true` | Show volume percentage in label |
| `max_volume` | u32 | `100` | Maximum volume cap (100 = normal, up to 150 for boost) |
| `scroll_step` | u32 | `5` | Volume change percentage per action |
| `show_microphone` | bool | `true` | Show microphone control item |
| `show_mic_volume` | bool | `true` | Show microphone volume percentage in label |
| `mic_max_volume` | u32 | `100` | Maximum mic volume cap (100 = normal, up to 150 for boost) |
| `mic_scroll_step` | u32 | `5` | Mic volume change percentage per action |

**Display (Output):**
- Label: Volume percentage (e.g., "75%") or "Muted"
- Tooltip: Volume %, mute status, output device name
- Icon: `audio-volume-muted`, `audio-volume-low`, `audio-volume-medium`, or `audio-volume-high`

**Display (Microphone):**
- Label: Volume percentage (e.g., "75%") or "Muted"
- Tooltip: Volume %, mute status, input device name
- Icon: `microphone-sensitivity-muted`, `microphone-sensitivity-low`, `microphone-sensitivity-medium`, or `microphone-sensitivity-high`

**Actions:**
- Enter/Click: Toggle mute
- Up/Down arrows: Adjust volume when focused on output or microphone item

**Requirements:** Requires `pactl` command (from `pulseaudio-utils` on Debian/Ubuntu or included with `pipewire-pulse`).

#### Power Profiles (`[modules.power_profiles]`)

Displays and controls system power profile via power-profiles-daemon. Allows switching between power-saver, balanced, and performance modes.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the power profiles module |

**Display:**
- Label: Current profile name (e.g., "Balanced")
- Tooltip: Profile name, degraded reason (if any)
- Icon: `power-profile-power-saver-symbolic`, `power-profile-balanced-symbolic`, or `power-profile-performance-symbolic`

**Actions:**
- Enter/Click: Cycle to next profile (Power Saver → Balanced → Performance → ...)
- Individual profile actions available in context menu

**Requirements:** Requires `power-profiles-daemon` to be installed and running. Available on most modern Linux distributions.

#### GPU (`[modules.gpu]`)

Displays GPU usage percentage and optionally temperature. Supports NVIDIA (via `nvidia-smi`) and AMD (via sysfs).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the GPU module |
| `show_temperature` | bool | `false` | Show GPU temperature in tooltip |
| `show_top_process` | bool | `false` | Show top GPU memory process in tooltip (NVIDIA only) |
| `interval_seconds` | u64 | `5` | Update interval in seconds |

**Display:**
- Label: GPU usage percentage (e.g., "GPU 45%")
- Tooltip: Usage %, temperature (if enabled), top process (if enabled)
- Icon: `video-display` (or `dialog-warning` if temperature ≥80°C)

**Supported GPUs:**
- **NVIDIA**: Full support via `nvidia-smi` (usage, temperature, top process)
- **AMD**: Usage and temperature via sysfs (`/sys/class/drm/card*/device/`)
- **Intel**: Detection only (usage monitoring requires elevated permissions)

**Requirements:** For NVIDIA GPUs, requires the proprietary NVIDIA driver with `nvidia-smi` available.

#### Scripts (`[[modules.scripts]]`)

Run custom scripts and display their output. Each script must be explicitly enabled for security. Multiple scripts can be configured using TOML array-of-tables syntax.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | string | required | Unique identifier for this script |
| `path` | string | required | Path to the script to execute |
| `enabled` | bool | `false` | **Must be `true`** to run (security measure) |
| `mode` | string | `"interval"` | Execution mode (see below) |
| `interval_seconds` | u64 | `30` | Update interval (only for `interval` mode) |
| `icon` | string | `null` | Default icon (can be overridden by script output) |

**Execution Modes:**

| Mode | Description |
|------|-------------|
| `once` | Run script once when module loads |
| `watch` | Spawn as long-running process, monitor stdout for updates (each line triggers update) |
| `interval` | Run script at regular intervals |
| `on_connect` | Run script when module starts and on config reload |

**Output Formats:**

Scripts can output in two formats (auto-detected based on whether output starts with `{`):

1. **JSON format** - for structured output with actions:
```json
{
  "label": "Display text",
  "tooltip": "Tooltip text",
  "icon": "icon-name",
  "actions": [
    {"id": "Activate", "command": "/path/to/click-handler.sh"},
    {"id": "ScrollUp", "command": "/path/to/scroll-up.sh"},
    {"id": "ScrollDown", "command": "/path/to/scroll-down.sh"}
  ]
}
```

2. **Line-based format** - simple text output:
```
Label text (first line)
Tooltip text (optional second line)
```

**Example Configuration:**

```toml
# Simple uptime display
[[modules.scripts]]
id = "uptime"
path = "/home/user/scripts/uptime.sh"
enabled = true
mode = "interval"
interval_seconds = 60
icon = "computer"

# Disk usage with click action
[[modules.scripts]]
id = "disk"
path = "/home/user/scripts/disk-usage.sh"
enabled = true
mode = "interval"
interval_seconds = 300
icon = "drive-harddisk"

# Long-running counter (watch mode)
[[modules.scripts]]
id = "counter"
path = "/home/user/scripts/counter.sh"
enabled = true
mode = "watch"
```

**Security:** Scripts are disabled by default and must have `enabled = true` explicitly set. The script path must exist or a warning is logged and the script is skipped.

**Example scripts** are available in the `examples/scripts/` directory.

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
│  │   ├─ GPU module (nvidia-smi / sysfs)                 │
│  │   ├─ Network module (interface stats from /sys)      │
│  │   ├─ Pipewire module (audio volume via pactl)        │
│  │   ├─ Power Profiles module (power-profiles-daemon)   │
│  │   ├─ Weather module (wttr.in API)                    │
│  │   └─ Scripts module (custom user scripts)            │
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
