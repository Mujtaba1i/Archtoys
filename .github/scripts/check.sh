#!/usr/bin/env bash
# Runs one CI check and reports it in plain words: a row in the run summary
# (what was checked, passed or failed), and for a failure the useful lines of
# the tool's output and how to fix it, also as an error annotation. Never a
# bare "exit code 1".
#
#   bash .github/scripts/check.sh "Title" "What it checks" "How to fix" [--keep REGEX] [--warn] -- command...
#
# --keep REGEX  only these output lines go in the report (the rest is in the log)
# --warn        a failure is a warning: reported, but the step still passes
set -uo pipefail
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

summary=${GITHUB_STEP_SUMMARY:-/dev/null}
if [ $code -eq 0 ]; then
  printf '| ✅ | **%s** | %s |\n' "$title" "$what" >> "$summary"
  exit 0
fi

lines="$( (if [ -n "$keep" ]; then grep -E "$keep" "$out"; else cat "$out"; fi) | sed 's/\x1b\[[0-9;]*m//g' | head -60)"
[ -n "$lines" ] || lines="$(sed 's/\x1b\[[0-9;]*m//g' "$out" | tail -30)"
mark="❌"; level=error
if [ $warn = 1 ]; then mark="⚠️"; level=warning; fi
{
  printf '| %s | **%s** | %s |\n\n' "$mark" "$title" "$what"
  printf '**%s: what to do.** %s\n\n' "$title" "$fix"
  printf '<details><summary>What it found</summary>\n\n```\n%s\n```\n</details>\n\n' "${lines//\`\`\`/\'\'\'}"
  printf '| | Check | What it checks |\n|---|---|---|\n'   # the next rows go in a fresh table
} >> "$summary"
first="$(printf '%s\n' "$lines" | grep -m1 -v '^[[:space:]]*$' | cut -c1-200)"
echo "::$level title=$title::$first. $fix"
[ $warn = 1 ] && exit 0
exit 1
