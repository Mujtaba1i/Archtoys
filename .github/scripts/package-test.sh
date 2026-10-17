#!/usr/bin/env bash
# Runs inside a clean distro container for package-check.yml.
#
#   package-test.sh install <channel>   put the published package on the system
#   package-test.sh start <channel>     the app must still run after 10 seconds
#
# Channels: appimage, deb, aur-bin, aur, rpm. Needs VERSION (0.3.2) and REPO.
# Every install checks it got exactly VERSION, so it can never test the old one.
set -euo pipefail

cmd="${1:?install or start}"
channel="${2:?channel}"
: "${VERSION:?}" "${REPO:?}"
want="$VERSION"
. /etc/os-release
VERSION="$want"  # os-release sets its own VERSION
fail() { echo "::error title=$channel on $PRETTY_NAME::$*"; exit 1; }

pkgs() {
  case "$ID" in
    ubuntu|debian) apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$@" >/dev/null ;;
    fedora) dnf install -y -q "$@" ;;
    arch) pacman -Syu --noconfirm --needed --quiet "$@" >/dev/null ;;
  esac
}

# Libraries the AppImage expects the host to have (see packaging/build-appimage.sh).
appimage_host_libs() {
  case "$ID" in
    ubuntu|debian) pkgs curl ca-certificates xvfb libgl1 libegl1 libgl1-mesa-dri libfontconfig1 libx11-6 libx11-xcb1 libxcb1 libwayland-client0 ;;
    fedora) pkgs curl xorg-x11-server-Xvfb mesa-libGL mesa-libEGL mesa-dri-drivers fontconfig libX11 libX11-xcb libxcb libwayland-client ;;
    arch) pkgs curl xorg-server-xvfb libglvnd mesa fontconfig libx11 libxcb wayland ;;
  esac
}

aur_install() {
  local name="$1"
  pkgs base-devel git sudo xorg-server-xvfb
  id builder >/dev/null 2>&1 || useradd -m builder
  echo "builder ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/builder
  sudo -u builder git clone -q "https://aur.archlinux.org/$name.git" "/home/builder/$name"
  local aur_ver
  aur_ver="$(sed -n 's/^pkgver=//p' "/home/builder/$name/PKGBUILD")"
  [ "$aur_ver" = "$VERSION" ] || fail "the AUR has $name $aur_ver, not $VERSION"
  (cd "/home/builder/$name" && sudo -u builder makepkg -si --noconfirm)
  pacman -Q "$name" | grep -q " $VERSION-" || fail "pacman did not install $name $VERSION"
}

rpm_install() {
  # dnf-command(copr) is dnf5-plugins on new Fedora, dnf-plugins-core on old
  pkgs 'dnf-command(copr)' xorg-x11-server-Xvfb
  dnf copr enable -y mujtaba1i/archtoys
  # COPR needs a little time to publish a build; wait until dnf sees it.
  for i in $(seq 1 30); do
    if dnf --refresh -q list --showduplicates archtoys 2>/dev/null | grep -q "[[:space:]]$VERSION-"; then
      dnf install -y "archtoys-$VERSION"
      [ "$(rpm -q --qf '%{VERSION}' archtoys)" = "$VERSION" ] || fail "dnf did not install archtoys $VERSION"
      return
    fi
    echo "COPR's repository does not list $VERSION yet ($i/30); waiting 30 s"
    sleep 30
  done
  fail "archtoys $VERSION never showed up in COPR's repository for $PRETTY_NAME (waited 15 minutes)"
}

install() {
  local url="https://github.com/$REPO/releases/download/v$VERSION"
  case "$channel" in
    appimage)
      appimage_host_libs
      curl -fsSL -o /opt/archtoys.AppImage "$url/Archtoys-$VERSION-x86_64.AppImage" \
        || fail "could not download Archtoys-$VERSION-x86_64.AppImage from the release"
      chmod +x /opt/archtoys.AppImage ;;
    deb)
      pkgs curl ca-certificates xvfb
      curl -fsSL -o /tmp/archtoys.deb "$url/archtoys_${VERSION}_amd64.deb" \
        || fail "could not download archtoys_${VERSION}_amd64.deb from the release"
      apt-get install -y -qq /tmp/archtoys.deb
      [ "$(dpkg-query -W -f='${Version}' archtoys)" = "$VERSION" ] || fail "apt did not install archtoys $VERSION" ;;
    aur-bin) aur_install archtoys-bin ;;
    aur) aur_install archtoys ;;
    rpm) rpm_install ;;
    *) fail "unknown channel $channel" ;;
  esac
}

start() {
  local app=(archtoys)
  if [ "$channel" = appimage ]; then
    export APPIMAGE_EXTRACT_AND_RUN=1  # containers have no FUSE
    app=(/opt/archtoys.AppImage)
  fi
  export HOME="$(mktemp -d)"
  Xvfb :99 -screen 0 800x600x24 >/dev/null 2>&1 &
  sleep 2
  DISPLAY=:99 "${app[@]}" > /tmp/smoke.log 2>&1 &
  local pid=$!
  sleep 10
  if kill -0 "$pid" 2>/dev/null; then
    echo "The app is still running after 10 s: OK"
    cat /tmp/smoke.log
  else
    cat /tmp/smoke.log
    fail "the installed app exited within 10 seconds (its output is above)"
  fi
}

case "$cmd" in
  install|start) "$cmd" ;;
  *) fail "unknown command $cmd" ;;
esac
