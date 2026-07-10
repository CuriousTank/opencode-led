# 🚦 opencode-traffic-light

**A tiny floating traffic light that tells you what your AI coder is doing — without you having to look.**

[![GitHub stars](https://img.shields.io/github/stars/CuriousTank/opencode-led?style=social)](https://github.com/CuriousTank/opencode-led/stargazers)
[![GitHub release](https://img.shields.io/github/v/release/CuriousTank/opencode-led?color=blue)](https://github.com/CuriousTank/opencode-led/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-Linux%20(X11)-orange)](#)

[English](./README.md) | [简体中文](./README.zh-CN.md)

---

### 👀 Preview

![preview](./assets/dynamic_bulbs_2x.gif)

---

### Why?

When you let an AI agent like [opencode](https://opencode.ai) run a long task, you keep switching back to the terminal just to check: *is it done yet? is it stuck waiting for me?*

**opencode-traffic-light** is a single glowing bulb that floats on top of every window:

- 🔴 **Red** — it's thinking / working (`session.status = busy`)
- 🟡 **Yellow** — it needs you (permission pending)
- 🟢 **Green** — done, idle, ship it 🎉

Glance at your desktop. Know instantly. Get back to whatever you were doing.

Each opencode session gets its own bulb — they pop in and out as sessions start and exit. Click a bulb to jump straight to that terminal.

> ⚠️ **Platform**: Currently tested on **Ubuntu 20.04 (X11)**. Windows / macOS are on the roadmap.

---

### 💖 Found it useful?

If this little light saves you some tab-switching, consider giving it a **⭐ Star** — it helps others discover it, and keeps the project alive.

[![Stargazers over time](https://starchart.cc/CuriousTank/opencode-led.svg)](https://starchart.cc/CuriousTank/opencode-led)

---

## Features

- **🔴🟡🟢 Real-time status** — Red (busy) / Yellow (needs input) / Green (idle), with a pulsing animation for active states.
- **📦 Dynamic bulb count** — Automatically tracks opencode process creation and termination. Each running opencode session gets its own bulb; bulbs appear and disappear in real time as sessions start and exit. No manual configuration needed.
- **🖱️ Click to raise terminal** — Click any bulb to instantly bring the corresponding opencode terminal window to the foreground (cross-workspace, via EWMH `_NET_ACTIVE_WINDOW`). The window is matched by walking the process tree (`/proc`) and scoring window titles.
- **💬 Hover tooltips** — Hover a bulb to see the session title and current status. Tooltips appear above the bulb row and stay stable while you hover.
- **🪟 Transparent & always-on-top** — Borderless, click-through (XShape input region), stays above all windows without blocking interaction.
- **✋ Draggable** — Drag any bulb to reposition the widget.
- **🎨 Custom icons** — Right-click any bulb → "Customize Icons" to open the settings panel. Drag your own images (PNG / JPG / **animated GIF**) onto each colour to replace the default bulbs. Want a beating heart for red, a bouncing dot for yellow, or a confetti animation for green? Just drop the file in.

## Demo

### 🔴 Thinking (Red)

![](assets/ask_2x.gif)

### 🟡 Asking for input (Yellow)

![](assets/choice_2x.gif)

### 🖱️ Click bulb to raise terminal

![](assets/pinned_window_2x.gif)

### 📦 Dynamic bulb tracking (sessions appear/disappear in real time)

![](assets/dynamic_bulbs_2x.gif)

### 🎨 Custom icons (drag your own images or GIFs)

![](assets/setting_icon_2x.gif)

## Architecture

```
opencode process                   Rust monitor process
┌─────────────────────┐            ┌──────────────────────────┐
│ plugin status-pusher│   HTTP     │ tiny_http (127.0.0.1:9912)│
│  ├ event:status     │ ──POST───→ │  ├ state machine store   │
│  └ event:permission │            │  └ eframe floating window │
└─────────────────────┘            │     red/yellow/green PNG  │
                                   └──────────────────────────┘
```

- The plugin (TypeScript, ~70 lines) is auto-loaded from opencode's `.opencode/plugin/` directory. It captures `session.status` / `permission.updated` events and POSTs them to the monitor.
- The monitor (Rust, egui/eframe rendering) listens on a local port and renders a borderless, transparent, always-on-top, draggable window — supporting multiple session bulbs simultaneously.

## Installation

### Option A: Install via .deb package (recommended)

Download the latest `.deb` from [GitHub Releases](https://github.com/CuriousTank/opencode-led/releases):

```bash
sudo dpkg -i opencode-traffic-light_*_amd64.deb
sudo apt-get install -f   # auto-resolve missing dependencies
```

### Option B: Build from source (requires Rust)

```bash
cd opencode-traffic-light
cargo build --release
# Binary: target/release/opencode-traffic-light
```

Build/runtime dependency: system OpenGL library (included in most distros). No gtk/webkit required.

### Install the opencode plugin

Copy `plugin/status-pusher.ts` to either location — opencode auto-discovers it:

- **Project-level**: `<project>/.opencode/plugin/status-pusher.ts`
- **Global**: `~/.config/opencode/plugin/status-pusher.ts`

> The plugin requires `@opencode-ai/plugin` (bundled with opencode by default).

If installed via `.deb`, the plugin is available at `/usr/share/opencode-traffic-light/plugin/status-pusher.ts`.

## Usage

```bash
# 1. Launch the monitor
opencode-traffic-light          # if installed via .deb
# or
./target/release/opencode-traffic-light  # if built from source

# 2. Use opencode normally (in a project with the plugin)
opencode
```

A traffic light window will appear:
- **Drag** any bulb to move the widget
- **Click** a bulb to raise its terminal window to the foreground
- **Hover** a bulb to see the session title and status
- **Right-click** to open the menu (customize icons / quit)

## Configuration

The default port is `9912`. Override via environment variable:

```bash
OPENCODE_TL_PORT=8899 opencode-traffic-light
```

The plugin reads the same `OPENCODE_TL_PORT` variable to determine which port to push to.

## Custom Icons

Right-click any bulb → **Customize Icons** to open the settings panel. Each status (Running / Need Manual Action / Completed) has its own card where you can:

- **Drag & drop** a PNG, JPG, or animated GIF onto the drop zone
- **Click** the drop zone to browse and select a file
- **Preview** the icon live (GIFs animate in the preview)
- **Reset** to revert to the default bulb

Custom icons are stored in `~/.config/opencode-traffic-light/icons/{red,yellow,green}/`.

## Protocol

```jsonc
// Monitor listens on 127.0.0.1:9912
// Plugin → Monitor
POST /status   { "session_id": "ses_xxx", "project": "/path", "state": "running|done|input" }
POST /remove   { "session_id": "ses_xxx" }
GET  /health   -> "ok"
```

`state` values: `running` (red) / `input` (yellow) / `done` (green).

## Roadmap

opencode-traffic-light stays a **minimal, focused** traffic light — no feature bloat. Progress:

**Done**
- [x] Multi-session bulbs (one per opencode session, auto appear/disappear)
- [x] Click bulb → raise the matching terminal window
- [x] Custom icons (drag your own PNG / JPG / animated GIF onto each colour)
- [x] Pulsing animation + hover tooltips + draggable widget
- [x] System tray icon (aggregate status + show/hide widget + quit)
- [x] Icon size selector (Small / Medium / Large)

**Planned**
- [ ] Auto-idle timeout (return to idle after N minutes with no updates)
- [ ] Sound notifications (chime on yellow / green)
- [ ] AppImage distribution (beyond `.deb`)
- [ ] GitHub Actions auto-release on tag
- [ ] Wayland support (currently X11 only)
- [ ] macOS / Windows support

> Want something else? [Open an issue](https://github.com/CuriousTank/opencode-led/issues/new) — we keep the scope tight but listen to real needs.

## License

MIT
