# Archtoys

Archtoys is a fast, system-wide color picker for Linux, inspired by PowerToys Color Picker.  
It is built to feel native on KDE Plasma (X11) and target Arch-based distros first.

## Features
- Global hotkey: `Meta + Ctrl + C`
- Live preview near the cursor with HEX value
- One-click pick
- Auto-copy or open details on pick
- Color history with quick recall
- Tray icon with Open/Quit
- Light/Dark theme

## Supported Platforms
- **Linux (X11)** — KDE Plasma on Arch/CachyOS and similar Arch-based distros
- **Wayland:** not supported (global capture is restricted)

If you are on Wayland, switch to a Plasma X11 session at the login screen.

## Install (Arch-based)
```bash
paru -S archtoys
```

## Usage
1. Press `Meta + Ctrl + C` or click **Pick**
2. Move the cursor to preview colors
3. Click to select

## Settings
- **Dark Mode**
- **Minimize on Pick**
- **Auto Copy**
- **Clear History**

## Troubleshooting
**Picker shows black / doesn’t move**  
You are likely on Wayland. Switch to an X11 session.

## License
MIT
