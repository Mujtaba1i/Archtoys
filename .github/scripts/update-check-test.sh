#!/usr/bin/env bash
# The update check against made-up releases (CI, before a release): a newer
# one is found and offered, the running version and an older one are not.
# The real update of each install is checked after a release (package-check).
#
#   bash .github/scripts/update-check-test.sh target/release/archtoys
set -euo pipefail
app="$(realpath "$1")"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
current="$("$app" --version | awk '{ print $2 }')"

release() {
  printf '{"tag_name":"v%s","html_url":"https://github.com/Mujtaba1i/Archtoys/releases/tag/v%s","body":"- Something new\\n","assets":[]}' "$1" "$1" > "$dir/release.json"
  ARCHTOYS_UPDATE_API="file://$dir/release.json" "$app" --update --dry-run
}

fail() { echo "::error title=Update check::$*"; exit 1; }

out="$(release 99.0.0)"
echo "$out"
grep -q "Archtoys 99.0.0 is out" <<< "$out" || fail "a newer release (99.0.0) wasn't offered"
grep -q "Download it from https://github.com/Mujtaba1i/Archtoys/releases/tag/v99.0.0" <<< "$out" \
  || fail "a build that isn't a package should point at the release page"

out="$(release "$current")"
grep -q "Up to date" <<< "$out" || fail "the running version ($current) was offered as an update"
out="$(release 0.0.1)"
grep -q "Up to date" <<< "$out" || fail "an older release (0.0.1) was offered as an update"
echo "The update check offers only a newer release: OK"
