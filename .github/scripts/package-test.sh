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
    ubuntu|debian) pkgs curl ca-certificates xvfb xdotool libgl1 libegl1 libgl1-mesa-dri libfontconfig1 libx11-6 libx11-xcb1 libxcb1 libwayland-client0 ;;
    fedora) pkgs curl xorg-x11-server-Xvfb xdotool mesa-libGL mesa-libEGL mesa-dri-drivers fontconfig libX11 libX11-xcb libxcb libwayland-client ;;
    # Arch's fontconfig brings no fonts; a real desktop always has some.
    arch) pkgs curl xorg-server-xvfb xdotool libglvnd mesa fontconfig ttf-dejavu libx11 libxcb wayland ;;
  esac
}

aur_install() {
  local name="$1"
  # base-devel and git build the package. The rest is what a desktop supplies
  # (a display, fonts, OpenGL), not something the package itself must pull in.
  pkgs base-devel git sudo xorg-server-xvfb xdotool ttf-dejavu mesa
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
  # The display, xdotool and the OpenGL driver are what a desktop supplies; the
  # libraries the app needs must come from the package itself.
  pkgs 'dnf-command(copr)' xorg-x11-server-Xvfb xdotool mesa-dri-drivers
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
      pkgs curl ca-certificates xvfb xdotool
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
  HOME="$(mktemp -d)"
  export HOME DISPLAY=:99
  Xvfb :99 -screen 0 800x600x24 >/dev/null 2>&1 &
  sleep 2
  "${app[@]}" > /tmp/smoke.log 2>&1 &
  local pid=$!

  # A window must appear: running is not enough, the app could hang before drawing.
  local window=""
  for _ in $(seq 1 20); do
    window="$(xdotool search --name '^Archtoys$' 2>/dev/null | head -n 1 || true)"
    [ -n "$window" ] && break
    sleep 0.5
  done
  sleep 8   # about 10 s in total
  if ! kill -0 "$pid" 2>/dev/null; then
    cat /tmp/smoke.log
    fail "the installed app exited within 10 seconds (its output is above)"
  fi
  if [ -z "$window" ]; then
    cat /tmp/smoke.log
    fail "the app is running but no window titled Archtoys appeared (its output is above)"
  fi
  echo "The app is running with its window after 10 s: OK"

  # One copy at a time: a second launch must end by itself and leave exactly one.
  timeout 30 "${app[@]}" > /tmp/second.log 2>&1 || { cat /tmp/second.log; fail "a second launch did not end by itself"; }
  cat /tmp/second.log
  local copies=0 dir name
  for dir in /proc/[0-9]*; do
    name = "$(cat "$dir/comm" 2>/dev/null || true)"
    case "$name" in
        archtoys|archtoys-bin) copies=$((copies + 1));;
    esac
  done
  [ "$copies" = 1 ] || fail "after a second launch $copies copies of archtoys are running; it must be exactly 1"
  echo "A second launch left exactly one copy running: OK"
  cat /tmp/smoke.log
}

case "$cmd" in
  install|start) "$cmd" ;;
  *) fail "unknown command $cmd" ;;
esac
