#!/usr/bin/env bash
# Picks colors on a virtual X11 screen split into two monitors, the way a
# user would: arrow keys, Enter, keypad Enter, a click and Esc, at the edge
# between the monitors and in the far corner. Checks the color the app saved.
#
#   bash .github/scripts/x11-pick-test.sh [path/to/archtoys]
#
# Needs Xvfb, xrandr, xdotool, feh, picom and dbus-run-session. picom is there
# because the picker's see-through input shield is black without a compositor.
set -euo pipefail

BIN=$(realpath "${1:-target/debug/archtoys}")
if [ -z "${ARCHTOYS_PICK_TEST_INNER:-}" ]; then
  # one session bus for the whole test, so `archtoys --pick` reaches the running copy
  ARCHTOYS_PICK_TEST_INNER=1 exec dbus-run-session -- bash "$0" "$BIN"
fi

WORK=$(mktemp -d)
export DISPLAY=:97 XDG_CONFIG_HOME="$WORK/config" XDG_RUNTIME_DIR="$WORK/run" XDG_SESSION_TYPE=x11
mkdir -p "$XDG_CONFIG_HOME/archtoys-color-picker" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
CONFIG="$XDG_CONFIG_HOME/archtoys-color-picker/config.json"
pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT

# Two 1280x1080 monitors side by side; red left, blue right; marked pixels at
# the edge between them (green | yellow) and in the bottom-right corner (cyan).
python3 - "$WORK/bg.png" <<'PY'
import struct, sys, zlib
W, H = 2560, 1080
marks = {(1279, 900): (0, 255, 0), (1280, 900): (255, 255, 0), (2559, 1079): (0, 255, 255)}
rows = []
for y in range(H):
    row = bytearray(b"\0")
    for x in range(W):
        row += bytes(marks.get((x, y), (200, 50, 50) if x < 1280 else (50, 50, 200)))
    rows.append(bytes(row))
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(b"".join(rows), 1)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY

Xvfb :97 -noreset -screen 0 2560x1080x24 >"$WORK/xvfb.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do xdotool getdisplaygeometry >/dev/null 2>&1 && break; sleep 0.1; done
# the two monitors (Xvfb runs with -noreset: it forgets them when its last client leaves)
xrandr --setmonitor LEFT 1280/340x1080/190+0+0 none
xrandr --setmonitor RIGHT 1280/340x1080/190+1280+0 none
xrandr --listmonitors
[ "$(xrandr --listmonitors | grep -c -E ' (LEFT|RIGHT) ')" -eq 2 ] || { echo "FAIL: no two monitors"; exit 1; }
picom --backend xrender >"$WORK/picom.log" 2>&1 & pids+=($!)
feh --no-fehbg --bg-center "$WORK/bg.png"

# "Minimize on Pick" keeps the app's own window out of the way.
echo '{"setting_minimize":true,"last_color":[1,2,3]}' >"$CONFIG"
"$BIN" >"$WORK/app.log" 2>&1 & pids+=($!)
sleep 3

last_color() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("last_color"))' "$CONFIG"; }
failed=0
# pick NAME EXPECTED X Y KEYS... : starts a pick, puts the mouse at (X, Y), presses KEYS
pick() {
  local name=$1 want=$2 x=$3 y=$4; shift 4
  "$BIN" --pick
  sleep 1.5
  xdotool mousemove "$x" "$y"; sleep 0.3
  for key in "$@"; do
    case $key in
      click) xdotool click 1 ;;
      Escape) xdotool keydown Escape; sleep 0.1; xdotool keyup Escape ;; # Esc is polled, not grabbed
      *) xdotool key "$key" ;;
    esac
    sleep 0.05
  done
  local got=""
  for _ in $(seq 30); do
    sleep 0.1
    got=$(last_color)
    [ "$got" = "$want" ] && break
  done
  # the pick must be over (Esc especially: the color stays the same either way)
  local open=""
  for _ in $(seq 20); do
    open=$(xdotool search --onlyvisible --name '^Archtoys Picker Shield$' 2>/dev/null || true)
    [ -z "$open" ] && break
    sleep 0.1
  done
  if [ -n "$open" ]; then
    echo "FAIL $name: the picker is still open"
    failed=1
  elif [ "$got" = "$want" ]; then
    echo "ok   $name: $got"
  else
    echo "FAIL $name: expected $want, got $got"
    failed=1
  fi
}

pick "arrows to the left monitor's last column, Enter" "[0, 255, 0]" 1270 900 \
  Right Right Right Right Right Right Right Right Right Return
pick "arrows across onto the right monitor, keypad Enter" "[255, 255, 0]" 1278 903 \
  Right Right Up Up Up KP_Enter
pick "arrows stop at the outer corner, click" "[0, 255, 255]" 2555 1075 \
  Right Right Right Right Right Down Down Down Down Down Down click
pick "Esc cancels" "[0, 255, 255]" 600 400 Left Escape
pick "a click on the left monitor" "[200, 50, 50]" 300 300 click

if [ "$failed" -ne 0 ]; then
  echo "--- app output ---"
  tail -n 40 "$WORK/app.log"
  exit 1
fi
echo "all picks right"
