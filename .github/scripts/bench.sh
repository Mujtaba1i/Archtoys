#!/usr/bin/env bash
# Measures Archtoys: binary size, time until the window shows, CPU used to
# start (in nanoseconds), what it reads from disk, idle CPU and wakeups,
# memory and threads, a color pick (`--pick`: time until the picker shows,
# memory during and after) and Text Extractor reading a fixed image
# (`--extract-file`: time, memory). Both builds first start once untimed (a
# fresh machine's font cache and desktop services: before v0.7.3 the build
# measured first paid for them alone, and v0.7.2's cold start showed 81-107%
# slower while every later start matched), then take turns for the cold
# starts (the disk cache dropped before each where sudo works without a
# password, as in CI). Given two binaries, it compares them and warns when the
# new one is clearly worse, and when it uses more than 100 MB. It never fails:
# it reports.
#
#   bash .github/scripts/bench.sh               # ./target/release/archtoys vs /usr/bin/archtoys
#   bash .github/scripts/bench.sh NEW OLD       # any two binaries (OLD is optional)
#   RUNS=5 bash .github/scripts/bench.sh        # runs per binary (default 3)
#   SEARCH=1 bash .github/scripts/bench.sh      # also the search speed test (from the repo, needs cargo)
#
# package-check.yml runs it on Xvfb after the package tests, on the published
# binary of each release against the previous release's; there the table and
# the warnings go to the run's summary page.
# Needs xdotool. Quit Archtoys first: only one copy can run at a time.

set -u
NEW=${1:-./target/release/archtoys}
OLD=${2:-/usr/bin/archtoys}
NEW_LABEL=${NEW_LABEL:-this build}
OLD_LABEL=${OLD_LABEL:-installed}
RUNS=${RUNS:-3}
SEARCH=${SEARCH:-0}
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

die() { echo "bench: $*" >&2; exit 1; }
command -v xdotool >/dev/null || die "install xdotool first (sudo dnf install xdotool)"
[ -x "$NEW" ] || die "no binary at $NEW (run cargo build --release --locked)"
ps -C archtoys -o stat= | grep -qv '^Z' && die "Archtoys is running. Quit it (tray icon > Quit) first."
[ -n "${DISPLAY:-}" ] || die "no X display (in CI, run it with xvfb-run)"

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }
# CPU time in nanoseconds, all of the app's threads (/proc/PID/task/*/schedstat).
# Before v0.7.3 this was counted in 10 ms ticks, too coarse to compare starts of
# 30 ms. Threads that already ended aren't counted; both builds alike.
cpu_ns() { cat /proc/"$1"/task/*/schedstat 2>/dev/null | awk '{ s += $1 } END { printf "%d", s }'; }
# How often the app woke up: all threads' context switches.
switches() { cat /proc/"$1"/task/*/status 2>/dev/null | awk '/^(non)?voluntary_ctxt_switches:/ { s += $2 } END { print s + 0 }'; }
kb() { awk -v k="$2" '$1 == k":" {print $2}' "/proc/$1/$3" 2>/dev/null; }

# One run. Prints: window_ms startup_cpu_ms idle_cpu(tenths of %) rss_kb pss_kb
# threads own_kb disk_read_kb peak_kb wakeups_per_s
# (-1 for what couldn't be measured; all -1 when the app quit or crashed). The
# app's own output goes to $OUT/app.log, so a failure can say why.
# mode x11: the window opens through X11 (XWayland on a Wayland desktop) so it
# can be timed; mode wayland: the normal way, untimed (Wayland hides windows
# from other apps).
FAILED_RUN="-1 -1 -1 -1 -1 -1 -1 -1 -1 -1"
run_once() {
  local bin=$1 mode=$2 pid t0 window=-1 i c_start c0 c1 w0 w1 read
  t0=$(now_ms)
  if [ "$mode" = x11 ]; then env -u WAYLAND_DISPLAY "$bin" >"$OUT/app.log" 2>&1 &
  else "$bin" >"$OUT/app.log" 2>&1 &
  fi
  pid=$!
  if [ "$mode" = x11 ]; then
    for i in $(seq 1 500); do  # up to 5 s
      xdotool search --onlyvisible --name '^Archtoys$' >/dev/null 2>&1 && { window=$(( $(now_ms) - t0 )); break; }
      kill -0 "$pid" 2>/dev/null || break  # it quit: no need to wait 5 s
      sleep 0.01
    done
  else
    sleep 1.5
  fi
  if ! kill -0 "$pid" 2>/dev/null; then  # it quit or crashed
    wait "$pid" 2>/dev/null
    echo "$FAILED_RUN"; return
  fi
  c_start=$(cpu_ns "$pid")
  read=$(awk '$1 == "read_bytes:" { print int($2 / 1024) }' "/proc/$pid/io" 2>/dev/null)
  sleep 2; c0=$(cpu_ns "$pid"); w0=$(switches "$pid"); sleep 5; c1=$(cpu_ns "$pid"); w1=$(switches "$pid")
  local rss pss threads own peak
  rss=$(kb "$pid" VmRSS status) pss=$(kb "$pid" Pss smaps_rollup) threads=$(kb "$pid" Threads status) peak=$(kb "$pid" VmHWM status)
  # own: memory no other process shares (the app's heap, not the libraries)
  own=$(awk '$1 == "Private_Clean:" || $1 == "Private_Dirty:" {s += $2} END {print s + 0}' "/proc/$pid/smaps_rollup" 2>/dev/null)
  echo "$window $(( c_start / 1000000 )) $(( (c1 - c0) / 5000000 )) ${rss:--1} ${pss:--1} ${threads:--1} ${own:--1} ${read:--1} ${peak:--1} $(( (w1 - w0) / 5 ))"
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; sleep 0.5
}

# A fresh machine's first app start builds the font cache (and loads every
# shared library from disk), and the first app to start also starts desktop
# services (the portal, D-Bus activation) that stay up for the next: all paid
# here, untimed, by both builds, before anything is measured. (v0.7.2 showed
# its cold start 81% and then 107% slower than v0.7.1 while every later start
# matched: the build measured first paid these alone.)
warm_system() {
  fc-list >/dev/null 2>&1
  local bin lib
  for bin in "$NEW" "$OLD"; do
    [ -x "$bin" ] || continue
    for lib in $(ldd "$bin" 2>/dev/null | awk '/=> \// {print $3}'); do cat "$lib" >/dev/null 2>&1; done
    run_once "$bin" x11 >/dev/null
  done
}
# Each cold start: drop the disk cache first (needs sudo without a password,
# as on CI runners). Elsewhere the libraries stay warm for both builds.
drop_disk_cache() {
  sync
  if sudo -n sh -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null; then
    echo "   (disk cache dropped: this start is cold)" >&2
  fi
}

median() { sort -n | awk '{ v[NR] = $1 } END { print NR ? v[int((NR + 1) / 2)] : -1 }'; }
# the average, leaving out what couldn't be measured (-1 if nothing could)
average() { awk '$1 >= 0 { s += $1; n++ } END { print n ? int(s / n + 0.5) : -1 }'; }

# cold KEYS...: COLD cold starts of each build, taking turns (new, old, then
# old, new, ...) so neither is always first; the median of each build's cold
# starts goes to KEY.window_first, KEY.startup_first and KEY.read_first.
COLD=${COLD:-3}
cold() {
  local round key keys=("$@") order
  for round in $(seq 1 "$COLD"); do
    if [ $((round % 2)) = 1 ]; then order=("${keys[@]}"); else order=(); for key in "${keys[@]}"; do order=("$key" "${order[@]}"); done; fi
    for key in "${order[@]}"; do
      echo "   $key, cold start $round of $COLD (about 10 s)" >&2
      drop_disk_cache
      run_once "${BIN[$key]}" x11 >> "$OUT/$key.cold"
      cp "$OUT/app.log" "$OUT/$key.log"
    done
  done
  for key in "${keys[@]}"; do
    awk '{ print $1 }' "$OUT/$key.cold" | median > "$OUT/$key.window_first"
    awk '{ print $2 }' "$OUT/$key.cold" | median > "$OUT/$key.startup_first"
    awk '{ print $8 }' "$OUT/$key.cold" | median > "$OUT/$key.read_first"
  done
}

# measure KEY BIN MODE: RUNS warm runs; KEY.<metric> is the median of each
# metric, except the start (window, startup): the average of the runs
measure() {
  local key=$1 bin=$2 mode=$3 r col names=(window startup idle rss pss threads own read peak wakeups)
  : > "$OUT/$key.runs"
  for r in $(seq 1 "$RUNS"); do
    echo "   $key, run $r of $RUNS ($mode, about 10 s)" >&2
    run_once "$bin" "$mode" >> "$OUT/$key.runs"
    cp "$OUT/app.log" "$OUT/$key.log"
  done
  for col in $(seq 1 10); do
    awk -v c=$col '{ print $c }' "$OUT/$key.runs" | median > "$OUT/$key.${names[col - 1]}"
  done
  for col in 1 2; do
    awk -v c=$col '{ print $c }' "$OUT/$key.runs" | average > "$OUT/$key.${names[col - 1]}"
  done
  du -k "$bin" | cut -f1 > "$OUT/$key.size"
}

# pick KEY BIN: `archtoys --pick` with no copy running starts in the tray and
# picks: the time until the picker shows (X11: its overlay window), the peak
# memory, and the memory 1.5 s after Esc closed it. PICKS runs, medians.
PICKS=${PICKS:-3}
pick() {
  local key=$1 bin=$2 r pid t0 shown i
  : > "$OUT/$key.picks"
  for r in $(seq 1 "$PICKS"); do
    echo "   $key, pick $r of $PICKS" >&2
    t0=$(now_ms); shown=-1
    env -u WAYLAND_DISPLAY "$bin" --pick >"$OUT/app.log" 2>&1 & pid=$!
    for i in $(seq 1 500); do
      xdotool search --onlyvisible --name '^Archtoys Picker' >/dev/null 2>&1 && { shown=$(( $(now_ms) - t0 )); break; }
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.01
    done
    sleep 0.5; xdotool key Escape; sleep 1.5
    echo "$shown $(kb "$pid" VmHWM status || echo -1) $(kb "$pid" VmRSS status || echo -1)" >> "$OUT/$key.picks"
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; sleep 0.5
  done
  awk '{ print ($1 == "" ? -1 : $1) }' "$OUT/$key.picks" | median > "$OUT/$key.pick"
  awk '{ print ($2 == "" ? -1 : $2) }' "$OUT/$key.picks" | median > "$OUT/$key.pick_peak"
  awk '{ print ($3 == "" ? -1 : $3) }' "$OUT/$key.picks" | median > "$OUT/$key.pick_after"
}

# text KEY BIN: Text Extractor reading a fixed image (`--extract-file`, the
# same engine as a capture): the time and the peak memory of the app itself
# (Tesseract runs as its own program). Needs Tesseract and GNU time; "—" without.
TEXT_IMAGE=${TEXT_IMAGE:-$(dirname "$0")/bench-text.png}
text() {
  local key=$1 bin=$2 r t0 ms out
  : > "$OUT/$key.texts"
  if ! command -v tesseract >/dev/null || [ ! -x /usr/bin/time ] || [ ! -f "$TEXT_IMAGE" ]; then
    echo -1 > "$OUT/$key.text"; echo -1 > "$OUT/$key.text_peak"; return
  fi
  "$bin" --extract-file "$TEXT_IMAGE" >/dev/null 2>&1  # untimed: the first read sets up the language data
  for r in 1 2 3; do
    t0=$(now_ms)
    /usr/bin/time -f "%M" -o "$OUT/time" "$bin" --extract-file "$TEXT_IMAGE" >/dev/null 2>&1
    ms=$(( $(now_ms) - t0 ))
    out=$(tail -1 "$OUT/time" 2>/dev/null)
    echo "$ms ${out:--1}" >> "$OUT/$key.texts"
  done
  awk '{ print $1 }' "$OUT/$key.texts" | median > "$OUT/$key.text"
  awk '{ print $2 }' "$OUT/$key.texts" | median > "$OUT/$key.text_peak"
}
get() { cat "$OUT/$1.$2"; }

HAVE_OLD=0
if [ -x "$OLD" ] && [ "$(readlink -f "$OLD")" != "$(readlink -f "$NEW")" ]; then HAVE_OLD=1; fi
declare -A BIN=([new]="$NEW" [old]="$OLD")
KEYS=(new); [ "$HAVE_OLD" = 1 ] && KEYS=(new old)
warm_system
cold "${KEYS[@]}"
for key in "${KEYS[@]}"; do
  measure "$key" "${BIN[$key]}" x11
  pick "$key" "${BIN[$key]}"
  text "$key" "${BIN[$key]}"
done
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
  measure new-wl "$NEW" wayland
  [ "$HAVE_OLD" = 1 ] && measure old-wl "$OLD" wayland
fi

# --- the report ---------------------------------------------------------------
# formatters; "—" for values that couldn't be measured (-1)
mb() { awk -v v="$1" 'BEGIN { if (v < 0) print "—"; else printf "%.1f MB", v / 1024 }'; }
ms() { [ "$1" -lt 0 ] && echo "—" || echo "$1 ms"; }
cpu() { [ "$1" -lt 0 ] && echo "—" || echo "$1 ms"; }
pct() { awk -v v="$1" 'BEGIN { if (v < 0) print "—"; else printf "%.1f %%", v / 10 }'; }
num() { [ "$1" -lt 0 ] && echo "—" || echo "$1"; }
change() { awk -v n="$1" -v o="$2" 'BEGIN { if (o <= 0 || n < 0) print "—"; else printf "%+.0f%%", (n - o) * 100 / o }'; }

WARNINGS=()
# warn NAME NEW OLD PERCENT MINIMUM: when NEW is above OLD by more than PERCENT
# and by more than MINIMUM (so small noisy numbers don't warn)
warn_if_worse() {
  awk -v n="$2" -v o="$3" -v p="$4" -v m="$5" 'BEGIN { exit !(o >= 0 && n > o * (1 + p / 100) && n - o > m) }' \
    && WARNINGS+=("$1 went from $6 to $7 ($(change "$2" "$3"))")
}

row() { # name formatter metric [key-suffix]
  local s=${4:-} n o
  n=$(get "new$s" "$3")
  if [ "$HAVE_OLD" = 1 ]; then o=$(get "old$s" "$3"); echo "| $1 | $($2 "$n") | $($2 "$o") | $(change "$n" "$o") |"
  else echo "| $1 | $($2 "$n") |"
  fi
}

{
  if [ "$HAVE_OLD" = 1 ]; then
    echo "| $COLD cold starts, $RUNS warm runs | $NEW_LABEL | $OLD_LABEL | change |"; echo "|---|---|---|---|"
  else
    echo "| $COLD cold starts, $RUNS warm runs | $NEW_LABEL |"; echo "|---|---|"
  fi
  row "Binary size" mb size
  row "Window shown after: cold start (median)" ms window_first
  row "Window shown after: warm start (average)" ms window
  row "CPU used to start: cold start (median)" cpu startup_first
  row "CPU used to start: warm start (average)" cpu startup
  row "Read from disk to start: cold start (median)" mb read_first
  row "Idle CPU (5 s)" pct idle
  row "Idle wakeups per second" num wakeups
  row "Memory (RSS)" mb rss
  row "Memory (PSS: shared libraries split between the apps using them)" mb pss
  row "Memory (own: used by nothing else)" mb own
  row "Peak memory while starting" mb peak
  row "Threads" num threads
  row "Pick: picker shown after (archtoys --pick)" ms pick
  row "Pick: peak memory" mb pick_peak
  row "Pick: memory after closing the picker" mb pick_after
  row "Text Extractor: reading a test image" ms text
  row "Text Extractor: peak memory (the app, not Tesseract)" mb text_peak
  if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    row "Wayland: idle CPU" pct idle -wl
    row "Wayland: memory (RSS)" mb rss -wl
  fi
} > "$OUT/table.md"

# A build whose window never showed: say what it printed (missing library, no
# font, crash), here and in the summary.
FAILED=()
for key in new old; do
  [ -f "$OUT/$key.window" ] && [ "$(get "$key" window_first)" -lt 0 ] && [ "$(get "$key" window)" -lt 0 ] || continue
  label=$NEW_LABEL; [ "$key" = old ] && label=$OLD_LABEL
  if [ "$(get "$key" rss)" -lt 0 ]; then
    WARNINGS+=("$label quit at startup instead of opening its window")
  else
    WARNINGS+=("the window of $label didn't show within 5 s")
  fi
  FAILED+=("$key")
done
if [ "$HAVE_OLD" = 1 ]; then
  n_size=$(get new size) o_size=$(get old size)
  warn_if_worse "Binary size" "$n_size" "$o_size" 5 256 "$(mb "$o_size")" "$(mb "$n_size")"
  warn_if_worse "Window shown after" "$(get new window)" "$(get old window)" 30 40 "$(get old window) ms" "$(get new window) ms"
  warn_if_worse "CPU used to start" "$(get new startup)" "$(get old startup)" 30 40 "$(get old startup) ms" "$(get new startup) ms"
  warn_if_worse "Idle CPU" "$(get new idle)" "$(get old idle)" 0 10 "$(pct "$(get old idle)")" "$(pct "$(get new idle)")"
  warn_if_worse "Memory (RSS)" "$(get new rss)" "$(get old rss)" 10 10240 "$(mb "$(get old rss)")" "$(mb "$(get new rss)")"
  warn_if_worse "Picker shown after" "$(get new pick)" "$(get old pick)" 30 40 "$(get old pick) ms" "$(get new pick) ms"
  warn_if_worse "Memory after a pick" "$(get new pick_after)" "$(get old pick_after)" 10 10240 "$(mb "$(get old pick_after)")" "$(mb "$(get new pick_after)")"
  warn_if_worse "Reading a test image" "$(get new text)" "$(get old text)" 30 150 "$(get old text) ms" "$(get new text) ms"
fi
# the goal since v0.6.2: under 100 MB while idle
if [ "$(get new rss)" -gt 102400 ]; then
  WARNINGS+=("Memory (RSS) of $NEW_LABEL is $(mb "$(get new rss)"), above the 100 MB goal")
fi

echo
cat "$OUT/table.md"
echo
if [ ${#WARNINGS[@]} -eq 0 ]; then
  [ "$HAVE_OLD" = 1 ] && echo "No big changes."
else
  printf 'WARNING: %s\n' "${WARNINGS[@]}"
fi
for key in "${FAILED[@]}"; do
  echo; echo "Output of the last $key run:"; tail -n 20 "$OUT/$key.log"
done

if [ "$SEARCH" = 1 ] && [ -f Cargo.toml ]; then
  echo; echo "Search speed (each keystroke must take under 2 ms):"
  search=$(cargo test --locked --release search::tests::speed -- --ignored --nocapture 2>/dev/null)
  grep -P '^\s*[0-9.]+ ms ' <<<"$search" | sort -rn | head -5
  grep '^worst' <<<"$search" || echo "   the speed test didn't run"
fi

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    if [ ${#WARNINGS[@]} -eq 0 ]; then echo "### ✅ Benchmark: $NEW_LABEL vs $OLD_LABEL"
    else echo "### ⚠️ Benchmark: $NEW_LABEL vs $OLD_LABEL"; fi
    echo; cat "$OUT/table.md"; echo
    for w in "${WARNINGS[@]}"; do echo "- ⚠️ $w"; done
    for key in "${FAILED[@]}"; do
      echo; echo "<details><summary>Output of the last $key run</summary>"; echo
      echo '```'; tail -n 20 "$OUT/$key.log"; echo '```'; echo "</details>"
    done
    echo; echo "_Both builds start once untimed first (font cache, desktop services), then take turns for the cold starts (disk cache dropped before each). Medians, except the warm start: the average of the warm runs. CPU is counted in nanoseconds. Warns when the new build is clearly worse: size +5%, warm start time or CPU +30% (and +40 ms), idle CPU +1 point, memory or memory after a pick +10% (and +10 MB), picker +30% (and +40 ms), reading the test image +30% (and +150 ms); and when it uses more than 100 MB (RSS). Shared runners are noisy: re-run the job before worrying._"
  } >> "$GITHUB_STEP_SUMMARY"
  for w in "${WARNINGS[@]}"; do echo "::warning title=Benchmark::$w"; done
fi
exit 0
