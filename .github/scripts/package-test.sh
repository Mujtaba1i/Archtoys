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
# Like fail, but puts the last lines of a log file inside the error message, so
# the real reason (not just "it failed") shows up in the annotation and the issue.
fail_log() {
  local file="$1" text
  shift
  text="$(tail -n 8 "$file" 2>/dev/null || true)"
  text="${text//$'\r'/}"
  text="${text//%/%25}"
  text="${text//$'\n'/%0A}"
  echo "::error title=$channel on $PRETTY_NAME::$*%0A$text"
  exit 1
}

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
  local n
  for n in 1 2 3 4 5; do
    rm -rf "/home/builder/$name"
    if sudo -u builder git clone -q "https://aur.archlinux.org/$name.git" "/home/builder/$name"; then
      break
    fi
    [ "$n" = 5 ] && fail "could not clone $name from the AUR after 5 tries"
    echo "AUR clone failed ($n/5); retrying in 15 s"
    sleep 15
  done
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
  # -noreset: an X server resets when its last client leaves and refuses
  # connections meanwhile; the app started right after the last one closed
  # sometimes hit that ("Failed to open connection to X server", v0.7.4)
  Xvfb :99 -screen 0 1280x1024x24 -noreset >/tmp/xvfb.log 2>&1 &
  local xvfb=$!
  local x
  for x in $(seq 1 40); do
    xdotool getdisplaygeometry >/dev/null 2>&1 && break
    kill -0 "$xvfb" 2>/dev/null || break
    sleep 0.5
  done
  xdotool getdisplaygeometry >/dev/null 2>&1 || { cat /tmp/xvfb.log; fail_log /tmp/xvfb.log "the virtual display (Xvfb) is not accepting connections:"; }
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
    fail_log /tmp/smoke.log "the installed app exited within 10 seconds. Its last output:"
  fi
  if [ -z "$window" ]; then
    cat /tmp/smoke.log
    fail_log /tmp/smoke.log "the app is running but no window titled Archtoys appeared. Its last output:"
  fi
  echo "The app is running with its window after 10 s: OK"

  # One copy at a time: a second launch must end by itself and leave exactly one.
  timeout 30 "${app[@]}" > /tmp/second.log 2>&1 || { cat /tmp/second.log; fail_log /tmp/second.log "a second launch did not end by itself. Its last output:"; }
  cat /tmp/second.log
  local copies=0 dir name
  for dir in /proc/[0-9]*; do
    name="$(cat "$dir/comm" 2>/dev/null || true)"
    # a zombie (state Z, ended but not yet reaped) isn't a running copy
    [ "$(awk '{print $3}' "$dir/stat" 2>/dev/null)" = Z ] && continue
    case "$name" in
        archtoys|archtoys-bin) copies=$((copies + 1));;
    esac
  done
  [ "$copies" = 1 ] || fail "after a second launch $copies copies of archtoys are running; it must be exactly 1"
  echo "A second launch left exactly one copy running: OK"
  cat /tmp/smoke.log
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true  # killed: not a failure
  # For the AppImage, $pid is only its launcher: stop the app itself too, or it
  # keeps the one-copy lock and the next start just exits (/proc: no pkill here)
  for dir in /proc/[0-9]*; do
    case "$(cat "$dir/comm" 2>/dev/null || true)" in
      archtoys|archtoys-bin) kill "${dir#/proc/}" 2>/dev/null || true ;;
    esac
  done
  sleep 1

  # The window keeps its size through closes when Archtoys is scaled on its
  # own (SLINT_SCALE_FACTOR=1.5 on a 100% desktop): resized to 1000 x 760, then
  # closed (the desktop's close request) and opened again (a second launch,
  # over D-Bus) 3 times, it must stay 1000 x 760. Before v0.7.2 it lost a third
  # each time (issue #11). A first start can't show it: Slint scales a size set
  # before the window exists right; the window shown again is what shrank.
  case "$ID" in
    ubuntu|debian) pkgs dbus python3-xlib ;;
    fedora) pkgs dbus-daemon python3-xlib ;;
    arch) pkgs dbus python-xlib ;;
  esac
  # the desktop's close button: WM_DELETE_WINDOW (xdotool's windowclose destroys the window instead)
  cat > /tmp/close.py <<'PY'
import sys
from Xlib import display, X, protocol
d = display.Display()
w = d.create_resource_object("window", int(sys.argv[1]))
proto, delete = d.intern_atom("WM_PROTOCOLS"), d.intern_atom("WM_DELETE_WINDOW")
w.send_event(protocol.event.ClientMessage(window=w, client_type=proto, data=(32, [delete, X.CurrentTime, 0, 0, 0])), event_mask=X.NoEventMask)
d.flush()
PY
  HOME="$(mktemp -d)"
  export APP="${app[*]}" SLINT_SCALE_FACTOR=1.5
  local size w h
  size="$(dbus-run-session -- bash -c '
    win() { xdotool search --onlyvisible --name "^Archtoys\$" 2>/dev/null | head -n 1; }
    $APP > /tmp/scaled.log 2>&1 &
    app=$!
    for _ in $(seq 1 20); do [ -n "$(win)" ] && break; sleep 0.5; done
    sleep 2
    xdotool windowsize "$(win)" 1000 760; sleep 1
    for _ in 1 2 3; do
      python3 /tmp/close.py "$(win)"; sleep 1.5
      timeout 30 $APP >/dev/null 2>&1; sleep 2
    done
    xdotool getwindowgeometry "$(win)" 2>/dev/null | awk "/Geometry/ {print \$2}"
    kill $app 2>/dev/null' || true)"
  unset SLINT_SCALE_FACTOR
  w="${size%x*}"; h="${size#*x}"
  if [ -z "$size" ] || [ "$w" -lt 998 ] || [ "$w" -gt 1002 ] || [ "$h" -lt 758 ] || [ "$h" -gt 762 ]; then
    cat /tmp/scaled.log
    fail_log /tmp/scaled.log "with SLINT_SCALE_FACTOR=1.5 the window resized to 1000x760 was ${size:-gone} after 3 closes and reopens; it must stay 1000x760 (issue #11). Its output:"
  fi
  echo "The window keeps its size through closes at SLINT_SCALE_FACTOR=1.5 ($size): OK"
}

case "$cmd" in
  install|start) "$cmd" ;;
  *) fail "unknown command $cmd" ;;
esac
