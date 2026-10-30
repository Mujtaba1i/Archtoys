Name:           archtoys
Version:        0.5.1
Release:        1%{?dist}
Summary:        A fast, system-wide color picker for Linux, inspired by PowerToys

License:        MIT
URL:            https://github.com/Mujtaba1i/Archtoys
Source0:        %{url}/archive/v%{version}/Archtoys-%{version}.tar.gz

# --- Core build toolchain ---
BuildRequires:  rust
BuildRequires:  cargo
BuildRequires:  pkgconf-pkg-config

# --- X11 / graphics libraries ---
BuildRequires:  libX11-devel
BuildRequires:  libxcb-devel
BuildRequires:  libXcursor-devel
BuildRequires:  libXi-devel
BuildRequires:  libXrandr-devel

# --- Font rendering ---
BuildRequires:  fontconfig-devel
BuildRequires:  freetype-devel

# --- OpenGL / GPU renderer (femtovg backend) ---
BuildRequires:  mesa-libGL-devel
BuildRequires:  mesa-libEGL-devel

# --- D-Bus (ksni tray + zbus portal) ---
BuildRequires:  dbus-devel

# --- Wayland ---
BuildRequires:  wayland-devel
BuildRequires:  wayland-protocols-devel
BuildRequires:  libxkbcommon-devel

# --- PipeWire (transitive dep pulled by scrap) ---
BuildRequires:  pipewire-devel

# --- Desktop integration ---
BuildRequires:  desktop-file-utils

Requires:       hicolor-icon-theme

# Libraries the app opens by name while it runs (winit and the renderer load
# them), so rpmbuild cannot see them in the binary. Written as sonames, so the
# package names do not matter. Without them a minimal install fails to start.
Requires:       libX11.so.6()(64bit)
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libXrender.so.1()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libEGL.so.1()(64bit)
Requires:       libGL.so.1()(64bit)
Recommends:     xdg-desktop-portal
Recommends:     mesa-dri-drivers

%description
Archtoys is a fast, system-wide color picker for Linux, inspired by PowerToys
Color Picker. It is built to feel native on KDE Plasma and targets both Wayland
and X11. Features include a configurable hotkey, smart input engine, light/dark
themes, color history, and a system tray icon.

%prep
%autosetup -n Archtoys-%{version}

# Vendor all crate dependencies so the offline mock buildroot can compile
# without network access.
cargo vendor --locked vendor
mkdir -p .cargo
cat > .cargo/config.toml << 'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
# Cargo.toml strips the binary; rpmbuild strips it itself and needs the symbols
# to make the debuginfo package, so keep them here.
cargo build --release --frozen --offline --config profile.release.strip=false

%install
rm -rf %{buildroot}

# Binary
install -Dm 0755 target/release/archtoys %{buildroot}%{_bindir}/archtoys

# .desktop file
install -Dm 0644 packaging/archtoys.desktop \
    %{buildroot}%{_datadir}/applications/archtoys.desktop

# Icons — hicolor theme, all available sizes
install -Dm 0644 packaging/archtoys-16.png  \
    %{buildroot}%{_datadir}/icons/hicolor/16x16/apps/archtoys.png
install -Dm 0644 packaging/archtoys-22.png  \
    %{buildroot}%{_datadir}/icons/hicolor/22x22/apps/archtoys.png
install -Dm 0644 packaging/archtoys-24.png  \
    %{buildroot}%{_datadir}/icons/hicolor/24x24/apps/archtoys.png
install -Dm 0644 packaging/archtoys-32.png  \
    %{buildroot}%{_datadir}/icons/hicolor/32x32/apps/archtoys.png
install -Dm 0644 packaging/archtoys-48.png  \
    %{buildroot}%{_datadir}/icons/hicolor/48x48/apps/archtoys.png
install -Dm 0644 packaging/archtoys-64.png  \
    %{buildroot}%{_datadir}/icons/hicolor/64x64/apps/archtoys.png
install -Dm 0644 packaging/archtoys-128.png \
    %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/archtoys.png
install -Dm 0644 packaging/archtoys-256.png \
    %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/archtoys.png
install -Dm 0644 packaging/archtoys-512.png \
    %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/archtoys.png

# Validate the installed .desktop entry
desktop-file-validate %{buildroot}%{_datadir}/applications/archtoys.desktop

%post
/usr/bin/update-desktop-database &>/dev/null || :
/usr/bin/gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor &>/dev/null || :

%postun
/usr/bin/update-desktop-database &>/dev/null || :
/usr/bin/gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor &>/dev/null || :

%files
%license LICENSE data/colornames-LICENSE.txt
%doc README.md
%{_bindir}/archtoys
%{_datadir}/applications/archtoys.desktop
%{_datadir}/icons/hicolor/16x16/apps/archtoys.png
%{_datadir}/icons/hicolor/22x22/apps/archtoys.png
%{_datadir}/icons/hicolor/24x24/apps/archtoys.png
%{_datadir}/icons/hicolor/32x32/apps/archtoys.png
%{_datadir}/icons/hicolor/48x48/apps/archtoys.png
%{_datadir}/icons/hicolor/64x64/apps/archtoys.png
%{_datadir}/icons/hicolor/128x128/apps/archtoys.png
%{_datadir}/icons/hicolor/256x256/apps/archtoys.png
%{_datadir}/icons/hicolor/512x512/apps/archtoys.png

%changelog
* Fri Oct 30 2026 Mujtaba1i <mujtaba1i@github> - 0.5.1-1
- fixed corrupted package-test script
- New HWB color format, typed values like hsl(50, 60%%, 70%%) stay as typed, the window remembers its size, the theme fades between light and dark, typing works right after Ctrl+F, the selected shade is marked, and history skips repeats
- Release notes list only the changes since the previous release, and every release is benchmarked against the previous one
- README: HWB and new screenshots
- Fixed the release script stopping before publishing

* Wed Oct 28 2026 Mujtaba1i <mujtaba1i@github> - 0.5.0-1
- UI finished
- README
- functionality DONE!
- ready to publish
- changed dependencies
- test
- v0.1.1 and fix AUR build
- v0.1.2
- fixed
- Fix light-mode text and icon install
- fixed a bug v0.1.5
- Release 0.1.6 with updated lockfile
- fix
- Fix tray icon + install proper icon sizes
- Add GitHub Actions release workflow
- Install fontconfig deps in CI
- Install X11 deps in CI
- Install xcb-shm deps in CI
- Fix release artifact name
- Grant release write permissions to workflow
- Fix picker overlay window lifecycle and release v0.1.8
- Fix settings panel overflow and release v0.1.9
- Prepare v0.2.1: tray-start fix, AppImage pipeline, repo cleanup
- Added Fedora Copr .spec file
- Copr build bug fix
- bug fix copr and rpm file workflow added
- bug fix copr
- bug fixes & copr release
- fix: vendor crates and fix spec for offline Copr buildroot
- ci: add manual trigger option for release-rpm workflow
- release-rpm fix
- modifified README.md
- Configure separate automated AUR deployment workflows
- Consolidate release workflows, fix AUR packaging, add one-time AUR hotfix
- Remove one-time AUR hotfix
- Build release binaries on ubuntu 22.04
- Bundle runtime and add smoke test
- Automate version bump, changelog and failure reports on release
- Add pixel magnifier and live preview on Wayland, desktop-managed hotkey, input validation, and fix color accuracy and crash bugs
- New README with screenshots
- Fixing README image missing
- README added aur link
- fix app icon wayland
- making releases wait for CI
- add color names, CMYK, HSLA, OKLCH and OKLab
- Color name license and readme update
- Follow COPR builds until the RPMs are attached
- New workflow to create .deb file
- fixed a bug with copr watcher in the CI
- new testing and releasing methods
- adding a report button and fixing a text color big
- The Fedora package now requires the libraries Archtoys loads while it runs
- Say so when no fonts or no browser are available, and keep the AppImage's libraries away from the browser
- Package checks now confirm a window appears and a second launch leaves one copy
- fixing package-test script to include archtoys-bin
- Added a search bar for name hex rgb and so, fixed a wayland pick bug, removed unused compiled crates
- Starting hidden at login no longer leaves a stuck window in the taskbar, and the tray icon appears even when the panel starts after Archtoys (#7)
- Minimize on Pick now keeps the window where you put it on KDE Plasma (Wayland)
- Fixed a bug in package-test for .deb:ubuntu24.04 and a bug for the last log lines not appearing in opened issue by the script
- New logo and app icon
- New look:first format in big on a color card, typing numbers like 255, 70, 162 in rgb is fine, named shades, a settings page, Light/Dark/System theme, and a tray dot with picked color
- CI reports how fast color search answers each keystroke
- README: new screenshots and the new look

* Mon Oct 26 2026 Mujtaba1i <mujtaba1i@github> - 0.4.1-1
- Starting hidden at login no longer leaves a stuck window in the taskbar, and the tray icon appears even when the panel starts after Archtoys (#7)
- Minimize on Pick now keeps the window where you put it on KDE Plasma (Wayland)

* Sat Oct 24 2026 Mujtaba1i <mujtaba1i@github> - 0.4.0-1
- fixing package-test script to include archtoys-bin
- Added a search bar for name hex rgb and so, fixed a wayland pick bug, removed unused compiled crates

* Sat Oct 17 2026 Mujtaba1i <mujtaba1i@github> - 0.3.3-1
- The Fedora package now requires the libraries Archtoys loads while it runs
- Say so when no fonts or no browser are available, and keep the AppImage's libraries away from the browser
- Package checks now confirm a window appears and a second launch leaves one copy

* Sat Oct 17 2026 Mujtaba1i <mujtaba1i@github> - 0.3.2-1
- fixed a bug with copr watcher in the CI
- new testing and releasing methods
- adding a report button and fixing a text color big

* Fri Oct 09 2026 Mujtaba1i <mujtaba1i@github> - 0.3.1-1
- Follow COPR builds until the RPMs are attached
- New workflow to create .deb file

* Fri Oct 02 2026 Mujtaba1i <mujtaba1i@github> - 0.3.0-1
- add color names, CMYK, HSLA, OKLCH and OKLab
- Color name license and readme update

* Fri Oct 02 2026 Mujtaba1i <mujtaba1i@github> - 0.2.7-1
- Fixing README image missing
- README added aur link
- fix app icon wayland
- making releases wait for CI

* Thu Oct 01 2026 Mujtaba1i <mujtaba1i@github> - 0.2.6-1
- Automate version bump, changelog and failure reports on release
- Add pixel magnifier and live preview on Wayland, desktop-managed hotkey, input validation, and fix color accuracy and crash bugs
- New README with screenshots

* Wed Sep 30 2026 Mujtaba1i <mujtaba1i@github> - 0.2.5-1
- Bundle runtime libraries in the AppImage and add a smoke test

* Wed Sep 30 2026 Mujtaba1i <mujtaba1i@github> - 0.2.4-1
- Build release binaries on Ubuntu 22.04

* Wed Sep 30 2026 Mujtaba1i <mujtaba1i@github> - 0.2.3-1
- Vendor crates and fix spec for offline Copr buildroot
- Add manual trigger option for release-rpm workflow
- Consolidate release workflows and fix AUR packaging

* Thu Jul 16 2026 Mujtaba1i <mujtaba1i@github> - 0.2.2-1
- Fix StartupWMClass and StartupNotify in .desktop for correct icon in KDE/GNOME
- Minimize-to-tray on close (no modal), new setting-minimize-tray toggle
- Fix ghost taskbar icon on KDE autostart via StartupNotify=false
- Install hicolor icons at all available sizes
- Add missing BuildRequires: mesa, dbus, wayland, freetype, libXi, libXrandr
- Vendor crate dependencies for offline mock buildroot compatibility

* Thu Jun 12 2025 Mujtaba1i <mujtaba1i@github> - 0.2.1-1
- Initial Fedora Copr release
