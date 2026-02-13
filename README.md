# Archtoys

Archtoys is a fast, system-wide color picker for Linux, inspired by PowerToys Color Picker.  
It is built to feel native on KDE Plasma and targets Arch-based distros first.

## Features
- Choose your own global hotkey with click-to-record capture (default: `Ctrl+Super+C`)
- One-click pick
- Auto-copy or open details on pick
- Color history with quick recall
- Tray icon with Open/Quit
- Light/Dark theme
- Autostart toggle (`--start-hidden` tray startup)
- Resizable window with enforced bounds: `480x320` to `900x620`

## Supported Platforms
- **Linux (X11)** — live cursor preview overlay + global picker.
- **Linux (Wayland)** — picker works via compositor/portal integration:
  - KWin DBus color picker (preferred on KDE)
  - XDG Desktop Portal `PickColor` fallback

Notes for Wayland:
- Live per-pixel hover preview overlay near the cursor is generally not available due compositor security limits.
- Behavior can vary by compositor and portal backend.
- Global hotkey registration depends on compositor/session support and may be unavailable.

## Install (Arch-based)
```bash
paru -S archtoys
```

Prebuilt package:
```bash
paru -S archtoys-bin
```

## Usage
1. Press your configured hotkey (default `Ctrl+Super+C`) or click **Pick**
2. Click a pixel to select

## Settings
- **Dark Mode**
- **Minimize on Pick**
- **Auto Copy**
- **Run on Startup**
- **Global Hotkey**: click the hotkey button, press your shortcut, and it saves immediately.
- Hotkeys must include at least one modifier: `Ctrl`, `Alt`, `Shift`, or `Super`.
- **Clear History**: clears history but keeps the currently selected color as the only history entry.

## Troubleshooting
**Wayland picker does nothing / closes**
- Ensure `xdg-desktop-portal` and a desktop-specific backend are installed and running.
- On KDE, make sure KWin DBus is available.
- If your compositor does not expose a picker portal/API, use an X11 session.

## License
MIT
