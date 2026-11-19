#!/usr/bin/env bash
# Runs one CI check and reports it in plain words. A failure goes in the run
# summary in the open: what was checked, the useful lines of the tool's
# output and how to fix it, also as an error annotation (never a bare "exit
# code 1"). A pass is only noted, and the job's last step folds them all
# into one line (v0.8.0: a page of passes hid the failures):
#
#   bash .github/scripts/check.sh "Title" "What it checks" "How to fix" [--keep REGEX] [--warn] -- command...
#   bash .github/scripts/check.sh --passed "Code quality"     (the job's last step)
#
# --keep REGEX  only these output lines go in the report (the rest is in the log)
# --warn        a failure is a warning: reported, but the step still passes
set -uo pipefail
summary=${GITHUB_STEP_SUMMARY:-/dev/null}
passes="${RUNNER_TEMP:-/tmp}/archtoys-checks-passed.md"
if [ "${1:-}" = "--passed" ]; then
  n=$( [ -f "$passes" ] && grep -c . "$passes" || echo 0)
  if [ "$n" -gt 0 ]; then
    { printf '<details><summary>✅ %s: %s check(s) passed (click to see them)</summary>\n\n' "$2" "$n"
      cat "$passes"; printf '\n</details>\n\n'; } >> "$summary"
  fi
  rm -f "$passes"
  exit 0
fi
title=$1 what=$2 fix=$3; shift 3
keep="" warn=0
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
  case $1 in
    --keep) keep=$2; shift 2 ;;
    --warn) warn=1; shift ;;
    *) echo "check.sh: unknown option $1" >&2; exit 2 ;;
  esac
done
shift  # the --
out="$(mktemp)"
"$@" > "$out" 2>&1
code=$?
cat "$out"

if [ $code -eq 0 ]; then
  printf -- '- ✅ **%s**: %s\n' "$title" "$what" >> "$passes"
  exit 0
fi

clean="$(sed 's/\x1b\[[0-9;]*m//g' "$out")"
kept="$(if [ -n "$keep" ]; then printf '%s\n' "$clean" | grep -E "$keep"; else printf '%s\n' "$clean"; fi)"
# errors first, so a long list of warnings can't push them out of the 60 lines
# (a block is an error/warning line and the lines under it)
lines="$(printf '%s\n' "$kept" | awk '
  /^(error|warning)/ { n++; err[n] = /^error/ }
  { block[n] = block[n] $0 "\n" }
  END { for (i = 0; i <= n; i++) if (err[i]) printf "%s", block[i]
        for (i = 0; i <= n; i++) if (!err[i]) printf "%s", block[i] }' | head -60)"
[ -n "$lines" ] || lines="$(printf '%s\n' "$clean" | tail -30)"
mark="❌"; level=error
if [ $warn = 1 ]; then mark="⚠️"; level=warning; fi
{
  printf '### %s %s\n\n%s\n\n' "$mark" "$title" "$what"
  printf '**What to do.** %s\n\n' "$fix"
  printf '<details><summary>What it found</summary>\n\n```\n%s\n```\n</details>\n\n' "${lines//\`\`\`/\'\'\'}"
} >> "$summary"
# the headline: the first real error and where it is (cargo's "-->", cargo deny's "┌─"),
# not whatever line came first (that was a warning's place, or a "│")
first="$(printf '%s\n' "$clean" | awk '
  /^error/ && !/^error: could not compile/ && !/^error: aborting/ && h == "" { h = $0; at = NR; next }
  h != "" && NR <= at + 3 && /(-->|┌─)/ { sub(/.*(-->|┌─) */, ""); sub(/^.*\/Archtoys\/Archtoys\//, ""); print h " (" $0 ")"; found = 1; exit }
  END { if (h != "" && !found) print h }')"
[ -n "$first" ] || first="$(printf '%s\n' "$lines" | grep -m1 -v '^[[:space:]]*$')"
first="$(printf '%s' "$first" | cut -c1-200)"
echo "::$level title=$title::$first. $fix"
[ $warn = 1 ] && exit 0
exit 1
