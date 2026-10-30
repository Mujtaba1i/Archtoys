#!/usr/bin/env bash
# Prints the commit that release TAG's changelog starts after: the previous
# release, i.e. the highest vX.Y.Z tag below TAG. Prints nothing when there is
# no earlier release. Usage: changes-base.sh TAG [COMMIT]  (COMMIT: TAG's commit)
#
# When main's history was rewritten after that release (for example to change
# the email address in every commit), its tag still points to the old copy of
# the commits, which isn't on main. Then the commit on main with the same files
# is used (or the "Release vX" commit with the same name). Without this, the
# v0.5.0 notes listed the whole history.
set -eu
TAG="$1"
HEAD="${2:-$TAG}"

PREV="$({ git tag -l 'v[0-9]*'; echo "$TAG"; } \
  | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V -u \
  | awk -v t="$TAG" '$0 == t { exit } { p = $0 } END { print p }')"
[ -n "$PREV" ] || exit 0

if git merge-base --is-ancestor "$PREV" "$HEAD"; then
  git rev-parse "$PREV^{commit}"
  exit 0
fi
TREE="$(git rev-parse "$PREV^{tree}")"
SAME="$(git log --format='%H %T' "$HEAD" | awk -v t="$TREE" '$2 == t { print $1; exit }')"
[ -n "$SAME" ] || SAME="$(git log --format='%H %s' "$HEAD" | awk -v s="Release $PREV" 'substr($0, 42) == s { print $1; exit }')"
echo "$SAME"
