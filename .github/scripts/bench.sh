#!/usr/bin/env bash
# Measures Archtoys: binary size, time until the window shows, CPU used to
# start, CPU when idle, memory and threads. Each run opens the app for about
# 10 seconds and closes it. Given two binaries, it compares them and warns when
# the new one is clearly worse. It never fails: it reports.
#
#   bash .github/scripts/bench.sh               # ./target/release/archtoys vs /usr/bin/archtoys
#   bash .github/scripts/bench.sh NEW OLD       # any two binaries (OLD is optional)
#   RUNS=5 bash .github/scripts/bench.sh        # runs per binary (default 3); medians are shown
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
TICK=$(getconf CLK_TCK)
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

die() { echo "bench: $*" >&2; exit 1; }
command -v xdotool >/dev/null || die "install xdotool first (sudo dnf install xdotool)"
[ -x "$NEW" ] || die "no binary at $NEW (run cargo build --release --locked)"
ps -C archtoys -o stat= | grep -qv '^Z' && die "Archtoys is running. Quit it (tray icon > Quit) first."
[ -n "${DISPLAY:-}" ] || die "no X display (in CI, run it with xvfb-run)"

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }
ticks() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
kb() { awk -v k="$2" '$1 == k":" {print $2}' "/proc/$1/$3" 2>/dev/null; }

# One run. Prints: window_ms startup_cpu_ms idle_cpu(tenths of %) rss_kb pss_kb threads
# (-1 for what couldn't be measured; all -1 when the app quit or crashed). The
# app's own output goes to $OUT/app.log, so a failure can say why.
# mode x11: the window opens through X11 (XWayland on a Wayland desktop) so it
# can be timed; mode wayland: the normal way, untimed (Wayland hides windows
# from other apps).
run_once() {
  local bin=$1 mode=$2 pid t0 window=-1 i c_start c0 c1
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
    echo "-1 -1 -1 -1 -1 -1"; return
  fi
  c_start=$(ticks "$pid")
  sleep 2; c0=$(ticks "$pid"); sleep 5; c1=$(ticks "$pid")
  local rss pss threads
  rss=$(kb "$pid" VmRSS status) pss=$(kb "$pid" Pss smaps_rollup) threads=$(kb "$pid" Threads status)
  echo "$window $(( c_start * 1000 / TICK )) $(( (c1 - c0) * 200 / TICK )) ${rss:--1} ${pss:--1} ${threads:--1}"
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; sleep 0.5
}

median() { sort -n | awk '{ v[NR] = $1 } END { print NR ? v[int((NR + 1) / 2)] : -1 }'; }

# measure KEY BIN MODE: writes $OUT/KEY.<metric> with the median of each metric
measure() {
  local key=$1 bin=$2 mode=$3 r col names=(window startup idle rss pss threads)
  : > "$OUT/$key.runs"
  for r in $(seq 1 "$RUNS"); do
    echo "   $key, run $r of $RUNS ($mode, about 10 s)" >&2
    run_once "$bin" "$mode" >> "$OUT/$key.runs"
    cp "$OUT/app.log" "$OUT/$key.log"
  done
  for col in 1 2 3 4 5 6; do
    awk -v c=$col '{ print $c }' "$OUT/$key.runs" | median > "$OUT/$key.${names[col - 1]}"
  done
  du -k "$bin" | cut -f1 > "$OUT/$key.size"
}
get() { cat "$OUT/$1.$2"; }

measure new "$NEW" x11
HAVE_OLD=0
if [ -x "$OLD" ] && [ "$(readlink -f "$OLD")" != "$(readlink -f "$NEW")" ]; then
  measure old "$OLD" x11; HAVE_OLD=1
fi
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
  measure new-wl "$NEW" wayland
  [ "$HAVE_OLD" = 1 ] && measure old-wl "$OLD" wayland
fi

# --- the report ---------------------------------------------------------------
# formatters; "—" for values that couldn't be measured (-1)
mb() { awk -v v="$1" 'BEGIN { if (v < 0) print "—"; else printf "%.1f MB", v / 1024 }'; }
ms() { [ "$1" -lt 0 ] && echo "did not show" || echo "$1 ms"; }
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
    echo "| Medians of $RUNS runs | $NEW_LABEL | $OLD_LABEL | change |"; echo "|---|---|---|---|"
  else
    echo "| Medians of $RUNS runs | $NEW_LABEL |"; echo "|---|---|"
  fi
  row "Binary size" mb size
  row "Window shown after" ms window
  row "CPU used to start" cpu startup
  row "Idle CPU (5 s)" pct idle
  row "Memory (RSS)" mb rss
  row "Memory (PSS)" mb pss
  row "Threads" num threads
  if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    row "Wayland: idle CPU" pct idle -wl
    row "Wayland: memory (RSS)" mb rss -wl
  fi
} > "$OUT/table.md"

# A build whose window never showed: say what it printed (missing library, no
# font, crash), here and in the summary.
FAILED=()
for key in new old; do
  [ -f "$OUT/$key.window" ] && [ "$(get "$key" window)" -lt 0 ] || continue
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
    echo; echo "_Warns when the new build is clearly worse: size +5%, start time or start CPU +30% (and +40 ms), idle CPU +1 point, memory +10% (and +10 MB). Shared runners are noisy: re-run the job before worrying._"
  } >> "$GITHUB_STEP_SUMMARY"
  for w in "${WARNINGS[@]}"; do echo "::warning title=Benchmark::$w"; done
fi
exit 0
