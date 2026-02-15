# Archtoys

Archtoys is a fast, system-wide color picker for Linux, inspired by PowerToys Color Picker.  
It is built to feel native on KDE Plasma and targets Arch-based distros first.

## Features
- Configurable hotkey (default: `Ctrl+Super+C`)
- One-click pick
- Auto-copy or open details on pick
- Color history with quick recall
- Light/Dark theme
- Autostart toggle

## Supported Platforms
- **Linux (X11)** — live cursor preview overlay + global picker.
- **Linux (Wayland)** — picker works via compositor/portal integration

Notes for Wayland:
- Live per-pixel hover preview overlay near the cursor is generally not available due security limitations.

## Install (Arch-based)
```bash
paru -S archtoys
```

Prebuilt package:
```bash
paru -S archtoys-bin
```

AppImage (any Linux distro):
1. Download `Archtoys-<version>-x86_64.AppImage` from Releases.
2. Run:
```bash
chmod +x Archtoys-<version>-x86_64.AppImage
./Archtoys-<version>-x86_64.AppImage
```

## Usage
1. Chose your hotkey (default `Ctrl+Super+C`) or click **Pick**
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
