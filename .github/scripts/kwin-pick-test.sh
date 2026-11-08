#!/usr/bin/env bash
# KDE Plasma (Wayland): the color picker's frozen screen gets ready on every
# monitor (issue #12: with two monitors KWin 6 put both picker windows on the
# first one, so the screens stayed black and clicks did nothing). Runs
# `archtoys --pick` on a headless KWin with 1 and 2 monitors. A headless KWin
# can't take screenshots, so the picker is handed a test image
# (ARCHTOYS_TEST_SCREENSHOT) the size of the monitors. Plasma 5 and 6 (CI runs
# both). (Not tested: two monitors with scaling; headless KWin lays scaled
# outputs on top of each other.)
#
#   bash .github/scripts/kwin-pick-test.sh path/to/archtoys
set -euo pipefail
BIN="$(readlink -f "${1:?the archtoys binary}")"
export BIN

desktop="KDE Plasma (Wayland)"
rows=()
failed=0
# a plain PNG of W x H (python's standard library only)
png() {
  python3 - "$1" "$2" "$3" <<'PY'
import struct, sys, zlib
w, h, path = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
line = b"\x00" + b"".join(bytes([(x * 7) % 256, 120, 200]) for x in range(w))
raw = line * h
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                       + chunk(b"IDAT", zlib.compress(raw, 1)) + chunk(b"IEND", b""))
PY
}

one() {  # one N: the picker on a headless KWin with N monitors
  local n=$1 runtime log
  runtime="$(mktemp -d)"; chmod 700 "$runtime"; log="$(mktemp)"
  png $((1920 * n)) 1080 /tmp/pick-test.png
  XDG_RUNTIME_DIR="$runtime" N="$n" LOG="$log" dbus-run-session -- bash -c '
    kwin_wayland --virtual --no-lockscreen --socket wayland-k --width 1920 --height 1080 --output-count "$N" > /tmp/kwin-pick.log 2>&1 &
    for _ in $(seq 1 40); do [ -S "$XDG_RUNTIME_DIR/wayland-k" ] && break; sleep 0.25; done
    kwin_wayland --version 2>/dev/null | head -1 > /tmp/kwin-version
    export WAYLAND_DISPLAY=wayland-k XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=KDE ARCHTOYS_TEST_SCREENSHOT=/tmp/pick-test.png
    unset DISPLAY
    HOME="$(mktemp -d)" timeout 8 "$BIN" --pick > "$LOG" 2>&1 || true'
  version="$(cat /tmp/kwin-version 2>/dev/null)"; desktop="KDE Plasma ${version#kwin } (Wayland)"
  local ready got
  ready="$(grep -ao "freeze picker: ready after [0-9]* ms on [0-9]* monitor" "$log" | head -1 || true)"
  if [ -n "$ready" ] && [ "$(grep -o "on [0-9]*" <<<"$ready" | grep -o "[0-9]*")" = "$n" ]; then
    got="ready after $(grep -o "[0-9]* ms" <<<"$ready")"
    rows+=("| $n monitor(s) | ready on $n | $got | ✅ |")
    echo "$desktop, $n monitor(s): $got: OK"
  else
    got="$(grep -ao "freeze picker: .*" "$log" | head -1 | cut -c16-200 || true)"
    rows+=("| $n monitor(s) | ready on $n | ${got:-nothing} | ❌ |")
    echo "::error title=Color picker, $desktop::$n monitor(s): the frozen screen didn't get ready (${got:-no picker output})"
    echo "--- Archtoys"; tail -20 "$log"
    failed=1
  fi
}

one 1
one 2

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### Color picker: $desktop $([ "$failed" = 0 ] && echo "✅ passed" || echo "❌ failed")"
    echo
    echo "The frozen screen must get ready on every monitor (issue #12: two monitors stayed black)."
    echo
    echo "| Monitors | Expected | Got | |"
    echo "|---|---|---|---|"
    printf "%s\n" "${rows[@]}"
    echo
  } >> "$GITHUB_STEP_SUMMARY"
fi
exit "$failed"
