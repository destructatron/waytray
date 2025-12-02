# WayTray

An accessible, compositor-agnostic system tray for Linux.

WayTray implements the [StatusNotifierItem (SNI)](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) specification with a daemon + client architecture. Unlike traditional system trays that sit permanently in a panel, WayTray's client opens as a regular window on demand, making it fully accessible to screen readers like Orca.

## Features

- **Compositor agnostic**: Works on any Wayland compositor (or X11)
- **Accessible**: Full keyboard navigation and screen reader support
  - Enter/Space to activate items
  - Shift+F10 or Menu key for context menus
  - Arrow keys to navigate
  - Escape to close
- **Daemon + client architecture**: Daemon caches tray items; client displays them on demand
- **Real-time updates**: Items refresh automatically when applications update their status

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

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  waytray-daemon (always running)                        │
│  ├─ StatusNotifierHost (receives items via D-Bus)       │
│  ├─ StatusNotifierWatcher (fallback if none exists)     │
│  ├─ Item cache with real-time updates                   │
│  └─ org.waytray.Daemon interface for clients            │
└─────────────────────────────────────────────────────────┘
                            ↕ D-Bus
┌─────────────────────────────────────────────────────────┐
│  waytray (GTK4 client, invoked on demand)               │
│  ├─ Queries daemon for current items                    │
│  ├─ Displays accessible list view                       │
│  └─ Forwards user actions to daemon                     │
└─────────────────────────────────────────────────────────┘
```

## License

MIT
