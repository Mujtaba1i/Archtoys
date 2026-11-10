#!/usr/bin/env bash
# KDE Plasma (Wayland): the main window opens where it was closed (issue #11).
# Runs Archtoys on a headless KWin with its KWin script (kwin.rs), moves the
# window like a user would, closes it, opens it again, and starts it fresh
# with a saved place. Plasma 5 and 6 (CI runs both). Fails when the window
# isn't put back. (No visible jump: KWin tells scripts about a new window in
# the same step it places it, before it draws a frame of it; a script can't
# observe drawing, so that comes from KWin's source, not this test.)
#
#   bash .github/scripts/kwin-place-test.sh path/to/archtoys
set -euo pipefail
BIN="$(readlink -f "${1:?the archtoys binary}")"
QDBUS="$(command -v qdbus-qt6 || command -v qdbus6 || command -v qdbus || true)"
[ -n "$QDBUS" ] || { echo "kwin-place-test: install qdbus"; exit 2; }
# the app must read the config.json each test writes under its own HOME:
# GitHub's runner sets XDG_CONFIG_HOME, which Archtoys would read instead
unset XDG_CONFIG_HOME
export BIN QDBUS XDG_RUNTIME_DIR="$(mktemp -d)"
chmod 700 "$XDG_RUNTIME_DIR"

exec dbus-run-session -- bash -c '
set -uo pipefail
desktop="KDE Plasma (Wayland)"
rows=()
# the run summary (CI): what was tested, on which Plasma, and each check passed or not
report() {
  local verdict=$1
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] || return 0
  {
    echo "### Window place: $desktop $verdict"
    echo
    echo "The main window must open where it was closed (issue #11), through Archtoys own KWin script."
    echo
    echo "| Check | Expected | Got | |"
    echo "|---|---|---|---|"
    printf "%s\n" "${rows[@]}"
    echo
  } >> "$GITHUB_STEP_SUMMARY"
}
check() {  # check NAME EXPECTED GOT: one row, and a failure when they differ
  if [ "$2" = "$3" ]; then rows+=("| $1 | $2 | $3 | ✅ |"); else rows+=("| $1 | $2 | ${3:-nothing} | ❌ |"); fail "$1: expected $2, got ${3:-nothing}"; fi
}
fail() {
  echo "::error title=Window place, $desktop::$*"
  report "❌ failed"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    { echo "<details><summary>KWin and Archtoys output</summary>"; echo; echo "\`\`\`"
      grep -av "qt\.\|kf\.\|kwin_libinput\|kwin_wayland_backend\|^\s*$" /tmp/kwin.log 2>/dev/null | tail -15; echo "---"; tail -15 /tmp/app.log 2>/dev/null
      echo "\`\`\`"; echo "</details>"; echo; } >> "$GITHUB_STEP_SUMMARY"
  fi
  echo "--- KWin"; tail -20 /tmp/kwin.log; echo "--- Archtoys"; tail -20 /tmp/app.log; exit 1; }
# script output (print) as debug lines on stderr: KWin 6 names its categories differently and Qt 6 logs to the journal
QT_FORCE_STDERR_LOGGING=1 QT_LOGGING_RULES="*.debug=true" kwin_wayland --virtual --no-lockscreen --socket wayland-k --width 1920 --height 1080 > /tmp/kwin.log 2>&1 &
for _ in $(seq 1 40); do [ -S "$XDG_RUNTIME_DIR/wayland-k" ] && break; sleep 0.25; done
[ -S "$XDG_RUNTIME_DIR/wayland-k" ] || fail "KWin did not start (no Wayland socket after 10 s)"
version="$(kwin_wayland --version 2>/dev/null | head -1)"; echo "KWin: $version"
desktop="KDE Plasma ${version#kwin } (Wayland)"
export WAYLAND_DISPLAY=wayland-k XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=KDE
unset DISPLAY

# a KWin script, once (each needs its own name)
kwin() { local n; n="probe$(date +%s%N)"; printf "%s\n" "$1" > "/tmp/$n.js"
  "$QDBUS" org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "/tmp/$n.js" "$n" >/dev/null
  "$QDBUS" org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null; sleep 1; }
WINDOWS="(workspace.windowList ? workspace.windowList() : workspace.clientList())"
MAIN="const w = $WINDOWS.find(c => c.caption === \"Archtoys\");"
where() { local tag; tag="WHERE$(date +%s%N)"
  kwin "$MAIN print(\"$tag \" + (w ? w.frameGeometry.x + \",\" + w.frameGeometry.y + \" \" + w.frameGeometry.width + \"x\" + w.frameGeometry.height : \"none\"));"
  grep -ao "$tag .*" /tmp/kwin.log | cut -d" " -f2- ; }  # -a: the debug log has a few binary bytes

# Issue #11, again: Archtoys scaled on its own (SLINT_SCALE_FACTOR=1.5) with a
# saved size. Slint could "correct" the window to the layout 520 px wide as it
# appeared, and Archtoys set the saved size again a moment later: the size
# popped, and on KDE the window crept to the top left. It must open at its saved
# size (700x640 at 1.5 = 1050x960, plus the KWin frame), keep its place and size
# through closes and reopens, and never show another size (v0.7.2 failed this
# every time: at the wrong size, or at the right one after showing 520 px first).
sized() {  # "x,y WxH" -> "its saved size", or the size it has
  local s=${1#* } w h; w=${s%x*}; h=${s#*x}
  if [ "${w:-0}" -ge 1048 ] && [ "${w:-0}" -le 1062 ] && [ "${h:-0}" -ge 958 ] && [ "${h:-0}" -le 1000 ]; then echo "its saved size"; else echo "${s:-nothing}"; fi; }
H3="$(mktemp -d)"; mkdir -p "$H3/.config/archtoys-color-picker"
echo "{\"last_page\":1,\"window_size\":[700,640],\"window_geometry\":[600,200,1050,996]}" > "$H3/.config/archtoys-color-picker/config.json"
# every size the window has from the moment KWin adds it, so a jump shows
kwin "(workspace.windowAdded || workspace.clientAdded).connect(w => { if (w.caption !== \"Archtoys\") return;
  print(\"SIZES \" + w.frameGeometry.width + \"x\" + w.frameGeometry.height);
  w.frameGeometryChanged.connect(() => print(\"SIZES \" + w.frameGeometry.width + \"x\" + w.frameGeometry.height)); });"
SLINT_SCALE_FACTOR=1.5 HOME="$H3" "$BIN" > /tmp/app.log 2>&1 & app=$!
sleep 4
first="$(where)"; echo "scaled 1.5, saved 700x640 at 600,200: $first"
check "Scaled 1.5 with a saved size: opens at" "its saved size" "$(sized "$first")"
for _ in 1 2 3; do
  kwin "$MAIN w.closeWindow();"; sleep 1.5
  SLINT_SCALE_FACTOR=1.5 HOME="$H3" "$BIN" >/dev/null 2>&1; sleep 3
done
got="$(where)"; echo "after 3 closes and reopens: $got"
check "Scaled 1.5, 3 closes and reopens: place" "600,200" "${got%% *}"
check "Scaled 1.5, 3 closes and reopens: size" "its saved size" "$(sized "$got")"
# never shown at another size, not even for a moment (4 openings)
seen="$(grep -ao "SIZES [0-9]*x[0-9]*" /tmp/kwin.log | cut -d" " -f2 | while read -r z; do sized "0,0 $z"; done | sort | uniq -c | sed "s/^ *//" | tr "\n" ";" )"
echo "sizes the window had: $seen"
other="$(grep -ao "SIZES [0-9]*x[0-9]*" /tmp/kwin.log | cut -d" " -f2 | while read -r z; do [ "$(sized "0,0 $z")" = "its saved size" ] || echo "$z"; done | sort -u | tr "\n" " ")"
check "Scaled 1.5: never shown at another size first" "no other size" "${other:-no other size}"
kill $app; sleep 1

H="$(mktemp -d)"; mkdir -p "$H/.config/archtoys-color-picker"; echo "{\"last_page\":1}" > "$H/.config/archtoys-color-picker/config.json"
HOME="$H" "$BIN" > /tmp/app.log 2>&1 & app=$!
sleep 4
echo "opened (KWin places it): $(where)"
# moved like a user would (the place only: Archtoys sets its own size)
kwin "$MAIN w.frameGeometry = {x: 400, y: 300, width: w.frameGeometry.width, height: w.frameGeometry.height};"
echo "moved to 400,300: $(where)"
kwin "$MAIN w.closeWindow();"; sleep 1
HOME="$H" "$BIN" >/dev/null 2>&1; sleep 3
got="$(where)"; echo "closed and opened again: $got"
check "Moved to 400,300, closed, opened again" "400,300" "${got%% *}"
kill $app; sleep 1

H2="$(mktemp -d)"; mkdir -p "$H2/.config/archtoys-color-picker"
echo "{\"last_page\":1,\"window_geometry\":[700,250,800,600]}" > "$H2/.config/archtoys-color-picker/config.json"
HOME="$H2" "$BIN" > /tmp/app.log 2>&1 & app=$!
sleep 4
got="$(where)"; echo "new start with 700,250 saved: $got"
check "Started with 700,250 saved in config.json" "700,250" "${got%% *}"
kill $app
report "✅ passed"
echo "The window opens where it was ($desktop): OK"
'
