#!/usr/bin/env bash
# First job of release.yml: waits until the CI workflow (ci.yml) has finished
# for the tagged commit, and stops the release unless CI passed. Nothing has
# been changed yet at this point, so a failed CI costs no version number.
set -euo pipefail

REPO="${GITHUB_REPOSITORY:?}"
SHA="${GITHUB_SHA:?}"          # the commit the tag points to
TAG="${GITHUB_REF_NAME:?}"
APPEAR_WAIT="${APPEAR_WAIT:-180}"     # seconds to wait for a CI run to show up
FINISH_WAIT="${FINISH_WAIT:-3600}"    # seconds to wait for it to finish
POLL="${POLL:-20}"

fail() { echo "::error title=Release stopped::$*"; exit 1; }

start=$SECONDS
echo "Waiting for CI on commit ${SHA:0:7} (tag $TAG)..."
while :; do
  json="$(gh api "repos/$REPO/actions/workflows/ci.yml/runs?head_sha=$SHA&per_page=10")"
  read -r status conclusion url < <(printf '%s' "$json" | python3 -c '
import json, sys
runs = json.load(sys.stdin).get("workflow_runs", [])
if not runs:
    print("none none none")
else:
    r = runs[0]   # newest first
    print(r["status"], r.get("conclusion") or "none", r["html_url"])
')
  waited=$((SECONDS - start))

  if [ "$status" = "none" ]; then
    if [ "$waited" -ge "$APPEAR_WAIT" ]; then
      fail "CI never ran for commit ${SHA:0:7}. Push your commits first (git push), wait for CI to go green, then re-run this release (Actions -> this run -> Re-run all jobs)."
    fi
    echo "  no CI run for this commit yet (${waited}s)..."
  elif [ "$status" != "completed" ]; then
    if [ "$waited" -ge "$FINISH_WAIT" ]; then
      fail "CI is still running after $((FINISH_WAIT / 60)) minutes: $url"
    fi
    echo "  CI is $status (${waited}s)... $url"
  else
    case "$conclusion" in
      success)
        echo "CI passed: $url"
        exit 0 ;;
      cancelled)
        fail "CI for this commit was cancelled (usually because a newer push replaced it): $url. Tag your newest commit instead." ;;
      *)
        fail "CI failed for this commit ($conclusion): $url. Nothing was released or changed. The issue this run opens shows how to fix it and retry with the same tag." ;;
    esac
  fi
  sleep "$POLL"
done
