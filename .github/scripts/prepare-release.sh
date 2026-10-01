#!/usr/bin/env bash
# Called by the "prepare" job of release.yml when a tag like v1.2.3 is pushed.
#
#  1. checks the tag is on the latest commit of main
#  2. sets the version in Cargo.toml, Cargo.lock and archtoys.spec
#  3. writes a %changelog entry from the commit messages since the last tag
#  4. commits "Release vX.Y.Z" to main and moves the tag onto that commit
#
# Outputs (to $GITHUB_OUTPUT): version, sha, notes
set -euo pipefail

TAG="${GITHUB_REF_NAME:?}"
PACKAGER="${PACKAGER:?}"
fail() { echo "::error title=Release stopped::$*"; exit 1; }

VERSION="${TAG#v}"
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || fail "Tag '$TAG' must look like v1.2.3. Delete it with: git push origin :refs/tags/$TAG"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git fetch --quiet origin main

TAG_SHA="$(git rev-parse "$TAG^{commit}")"
MAIN_SHA="$(git rev-parse origin/main)"
# Tag must be on main's newest commit (or ahead of it, if main wasn't pushed yet).
git merge-base --is-ancestor "$MAIN_SHA" "$TAG_SHA" \
  || fail "Tag $TAG is not on the latest commit of main. Tag your newest commit: git tag -f $TAG && git push -f origin $TAG"

git checkout --quiet "$TAG_SHA"

CURRENT="$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)"
NEWEST="$(printf '%s\n%s\n' "$CURRENT" "$VERSION" | sort -V | tail -n 1)"
[ "$NEWEST" = "$VERSION" ] \
  || fail "Tag $TAG is older than the version already in Cargo.toml ($CURRENT). Use a higher version."

PREV="$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$TAG_SHA^" 2>/dev/null || true)"
RANGE="${PREV:+$PREV..}$TAG_SHA"
echo "Version: $CURRENT -> $VERSION   (changes since: ${PREV:-the beginning})"

# Commit subjects since the previous tag: no merges, no "Release vX" commits,
# each distinct message once, in the order they were made.
CHANGES="$(git log --reverse --no-merges --format='%s' "$RANGE" \
  | sed -e 's/[[:space:]]*$//' \
  | grep -v -E '^Release v[0-9]' \
  | grep -v -E '^[[:space:]]*$' \
  | awk '!seen[$0]++' || true)"
[ -n "$CHANGES" ] || CHANGES="Maintenance release"
echo "Changelog:"; printf '%s\n' "$CHANGES" | sed 's/^/  - /'

# --- Cargo.toml + Cargo.lock: the archtoys package's own version only -------
python3 - "$VERSION" <<'PY'
import re, sys
v = sys.argv[1]
s = open("Cargo.toml").read()
s, n = re.subn(r'(?m)^(\[package\][^\[]*?^version\s*=\s*)"[^"]*"', r'\g<1>"%s"' % v, s, count=1)
if n != 1: sys.exit("Could not find version in [package] of Cargo.toml")
open("Cargo.toml", "w").write(s)

s = open("Cargo.lock").read()
s, n = re.subn(r'(\[\[package\]\]\nname = "archtoys"\nversion = )"[^"]*"', r'\g<1>"%s"' % v, s, count=1)
if n != 1: sys.exit('Could not find the "archtoys" package in Cargo.lock')
open("Cargo.lock", "w").write(s)
PY

# --- archtoys.spec: Version, Release, and a new %changelog entry ------------
sed -i -E "s/^(Version:[[:space:]]+).*/\1${VERSION}/; s/^(Release:[[:space:]]+)[0-9]+/\11/" archtoys.spec
if grep -qE "^\* .* - ${VERSION//./\\.}-1$" archtoys.spec; then
  echo "archtoys.spec already has a changelog entry for $VERSION; leaving it."
else
  DATE="$(LC_ALL=C date -u '+%a %b %d %Y')"
  {
    echo "* $DATE $PACKAGER - ${VERSION}-1"
    printf '%s\n' "$CHANGES" | sed -e 's/%/%%/g' -e 's/^/- /'   # % is special in specs
    echo
  } > entry.txt
  awk 'FNR==NR { e = e $0 "\n"; next }
       { print }
       /^%changelog[[:space:]]*$/ && !done { printf "%s", e; done = 1 }' entry.txt archtoys.spec > spec.new
  mv spec.new archtoys.spec
  rm entry.txt
fi

git --no-pager diff --stat
if git diff --quiet; then
  echo "Files already say $VERSION; nothing to commit."
else
  git commit --quiet -am "Release $TAG"
  git push --quiet origin "HEAD:refs/heads/main" \
    || fail "Could not push the release commit to main (did main change during the release?). Re-run this workflow."
fi

SHA="$(git rev-parse HEAD)"
if [ "$SHA" != "$TAG_SHA" ]; then
  git tag -f "$TAG" "$SHA" >/dev/null
  git push --quiet -f origin "refs/tags/$TAG"
  echo "Moved tag $TAG to the release commit $SHA"
fi

{
  echo "version=$VERSION"
  echo "sha=$SHA"
  echo "notes<<NOTES_EOF"
  echo "## Changes${PREV:+ since $PREV}"
  echo
  printf '%s\n' "$CHANGES" | sed 's/^/- /'
  if [ -n "$PREV" ]; then
    echo
    echo "**Full list of changes:** https://github.com/${GITHUB_REPOSITORY}/compare/${PREV}...${TAG}"
  fi
  echo "NOTES_EOF"
} >> "$GITHUB_OUTPUT"
