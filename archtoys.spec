Name:           archtoys
Version:        0.2.2
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
cargo build --release --frozen --offline

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
%license LICENSE
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
* Wed Jul 16 2026 Mujtaba1i <mujtaba1i@github> - 0.2.2-1
- Fix StartupWMClass and StartupNotify in .desktop for correct icon in KDE/GNOME
- Minimize-to-tray on close (no modal), new setting-minimize-tray toggle
- Fix ghost taskbar icon on KDE autostart via StartupNotify=false
- Install hicolor icons at all available sizes
- Add missing BuildRequires: mesa, dbus, wayland, freetype, libXi, libXrandr
- Vendor crate dependencies for offline mock buildroot compatibility

* Thu Jun 12 2025 Mujtaba1i <mujtaba1i@github> - 0.2.1-1
- Initial Fedora Copr release