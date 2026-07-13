Name:           archtoys
Version:        0.2.1
Release:        1%{?dist}
Summary:        A fast, system-wide color picker for Linux,  inspired by PowerToys

License:        MIT
URL:            https://github.com/Mujtaba1i/Archtoys
Source0:        %{url}/archive/v%{version}/Archtoys-%{version}.tar.gz

BuildRequires:  rust
BuildRequires:  cargo
BuildRequires:  libX11-devel
BuildRequires:  libxcb-devel
BuildRequires:  libXcursor-devel
BuildRequires:  pipewire-devel
BuildRequires:  fontconfig-devel
BuildRequires:  pkgconf-pkg-config

%description
Archtoys is a fast, system-wide color picker for Linux, inspired by PowerToys 
Color Picker. It is built to feel native on KDE Plasma and targets both Wayland 
and X11. Features include a configurable hotkey, smart input engine, light/dark 
themes, and a color history.

%prep
%autosetup -n Archtoys-%{version}

%build
cargo build --release

%install
mkdir -p %{buildroot}%{_bindir}
install -m 0755 target/release/archtoys %{buildroot}%{_bindir}/archtoys

%files
%{_bindir}/archtoys
%license LICENSE
%doc README.md

%changelog
* Tue Jul 14 2026 Mujtaba1i - 0.2.1-1
- Initial Fedora Copr release