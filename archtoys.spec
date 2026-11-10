Name:           archtoys
Version:        0.7.3
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
# Text Extractor (v0.7.0); Archtoys can also install it when it is turned on
Suggests:       tesseract
Suggests:       tesseract-langpack-eng

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

# what KDE Discover and GNOME Software show
install -Dm 0644 packaging/io.github.Mujtaba1i.Archtoys.metainfo.xml \
    %{buildroot}%{_metainfodir}/io.github.Mujtaba1i.Archtoys.metainfo.xml

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
%{_metainfodir}/io.github.Mujtaba1i.Archtoys.metainfo.xml
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
* Tue Nov 10 2026 Mujtaba1i <mujtaba1i@github> - 0.7.3-1
- Tests: the appimage package check closes the app not the launcher
- The window opens at its saved size straight away when Archtoys is scaled on its own (SLINT_SCALE_FACTOR), instead of jumping to it, which on KDE could move it (issue #11). Report a problem counts each screen once and names the picker used last time. The menu button lines up with the sidebar's icons and the page name with the page. Buy me a coffee in Settings: a one-time tip on Ko-fi
- Tests: libraries with known security holes fail CI (cargo deny advisories)
- KDE Discover and GNOME Software show Archtoys' description, a screenshot and a Donate link; supporters who choose to be public are thanked in SUPPORTERS.md; releases come with SHA256SUMS
- Tests: CI checks formatting, common mistakes, libraries with known security holes, the software-center file and the support page, each explaining what failed; failure issues show the job's log again; a fairer benchmark with more numbers, also for pull requests; Dependabot
- README: Buy me a coffee, v0.7.3 done, the font finder moves to v0.7.4, then the Screen Ruler and Awake
- Tests: the window-place test waits longer
- fixed 2 known security holes (webbrowser , pulldown-cmark)

* Sun Nov 08 2026 Mujtaba1i <mujtaba1i@github> - 0.7.2-1
- CI: a shorter text extractor report
- The color picker works again on two or more monitors on KDE Plasma (Wayland): the screens no longer stay black (issue #12). Report a problem also adds Archtoys' last messages, and fills in the desktop's version, the screens and their scale, Archtoys' own scale, CPU or OpenGL, and Text Extractor's state. archtoys --version (or -v) prints the version. The window keeps its size and its place: with Archtoys scaled on its own (SLINT_SCALE_FACTOR) it no longer gets a third smaller each time it is opened again, and on KDE Plasma (Wayland) it opens where you left it, also after a restart. Drag the bar at the top to move the window, for desktops that show no title bar (issue #11)
- Tests: on KDE Plasma 5 and 6 (Wayland) the window must open where it was closed and the color picker must get ready on 1 and 2 monitors (a new CI job); package checks: the window must keep its size through closes and reopens with SLINT_SCALE_FACTOR=1.5, and zombie processes no longer count as running copies
- CI e2e test (NOT IMPLEMENTED YET WILL BE IMPLEMENTED WHEN THE DEVELOPER WANT TO!)
- README: v0.7.1 and v0.7.2 done, the font finder moves to v0.7.3; Text Extractor uses the internet only to download languages; moving the window by its top bar
- Fixed window for the window to open where is closed on slow computer (KDE Plasma, Wayland)
- the window opens where is closes
- the color picker no longer freezes with two or more moniters
- Tests: the window-place test reads the settings it writes

* Fri Nov 06 2026 Mujtaba1i <mujtaba1i@github> - 0.7.1-1
- Text Extractor reads other languages far better: 98%% of the text right across 24 languages (80%% before), from a word to a paragraph, in images and headings too; Arabic, Korean, Thai, Hindi, Bengali, Urdu and Chinese went from under 65%% to over 93%%. The first time it meets text that isn't clearly English it downloads its languages once (about 32 MB). Archtoys is also 12%% smaller and starts with less memory and CPU
- Benchmark: both builds start equally cold (the first build measured paid a fresh machine's one-off costs, which showed as a slow first start)
- README: a Text Extractor section with pictures, which languages it reads, Home; roadmap: v0.7.1 improvements, the font finder in v0.7.2
- README: updated old images to the new interface redesign

* Thu Nov 05 2026 Mujtaba1i <mujtaba1i@github> - 0.7.0-1
- Text Extractor, the second toy: drag over any text on screen to copy it, on every monitor, Wayland and X11; it finds the language by itself and downloads a language's data the first time it's needed, shows a toolbar on the selection (Copy, Copy as one line, Open) or copies and closes at once, and keeps a history of the last 500 texts with their language, how sure it is and the text size. It's off until you press Turn on, which installs Tesseract with your password on Arch, Fedora, Debian, Ubuntu and their families. Hotkey Ctrl+Super+T, or archtoys --extract. Enter copies at any moment, and only what you copy or open goes to the history. Your own languages (system language, keyboard layouts, input methods) are read first, so Arabic, Korean and mixed text read right; Chinese, Japanese and Thai keep their text unspaced. Archtoys now has a sidebar (only its icons, or all of it) and a Home page with your recent color and text to move between toys, settings for the app and each toy, and an About page with credits and every open-source licence. Color history keeps the last 500 colors
- CI: checks Text Extractor's accuracy, tests Turn on on Arch, Fedora and Debian, checks every dependency's licence; releases include the licence list
- README: Text Extractor, the sidebar and Home, and the roadmap
- CI: Arabic below 90%% gives warning not a failure

* Wed Nov 04 2026 Mujtaba1i <mujtaba1i@github> - 0.6.2-1
- The release benchmark installs the app's libraries and shows why a build didn't start
- Color editor with compare: the pen on the middle shade opens it to change saturation, brightness and hue or type in your first two formats, and Compare puts a picked or typed color next to yours with their contrast ratio. Clicking anywhere on a format box now puts the cursor in it
- Tests: pick colors on a virtual screen with two monitors in CI
- Picking works on every monitor: on Wayland the frozen screenshot covers all your monitors (also with different scaling per monitor) and the magnifier shows at once; on X11 the preview stays on the monitor you pick on. The arrow keys move the pick one pixel and Enter picks it, and the value next to the cursor is in your first format. Lighter and quicker: the window is now drawn on the CPU instead of with OpenGL, so Archtoys uses about a third of the memory (about 35 MB), starts in half the time and looks the same; the color names and the search index come ready-built instead of being worked out at every start, and memory the app no longer needs goes back to the system. Also: the color editor's Cancel and Use buttons and its two format boxes are the same width, on black or white only the middle shade has the pen, the little square in the picker's magnifier is sharp, and no thin dark lines between the shades
- Benchmark: shows the first (cold) start apart from the later ones and the app's own memory, and warns above 100 MB; the search speed test takes the median of 7 timings
- README: the color editor, picking on every monitor and with the arrow keys, and the window drawn on the CPU (and how to build the OpenGL version)
- New README images

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
