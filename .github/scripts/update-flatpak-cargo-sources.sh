#!/usr/bin/env bash
set -Eeuo pipefail

# Regenerate packaging/flatpak/cargo-sources.json from the Cargo.lock produced
# by prepare-release.sh, commit it to main, and move the release tag to the same
# commit. The main-branch push uses MAIN_PUSH_KEY to pass the Require CI rule;
# the tag update uses GITHUB_TOKEN so it does not start another workflow run.

: "${MAIN_PUSH_KEY:?MAIN_PUSH_KEY secret is required}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"
: "${PREPARE_SHA:?PREPARE_SHA is required}"
: "${RELEASE_TAG:?RELEASE_TAG is required}"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

HEAD_SHA="$(git rev-parse HEAD)"
if [[ "$HEAD_SHA" != "$PREPARE_SHA" ]]; then
  echo "::error::The checked-out commit ($HEAD_SHA) does not match prepare's output ($PREPARE_SHA)."
  exit 1
fi

TAG_REF="refs/tags/${RELEASE_TAG}"
MAIN_REF="refs/remotes/origin/main"

# Fetch the exact refs this job is going to update/verify.
git fetch --no-tags origin "+refs/heads/main:${MAIN_REF}"
git fetch --no-tags origin "+${TAG_REF}:${TAG_REF}"

REMOTE_MAIN_SHA="$(git rev-parse "$MAIN_REF")"
REMOTE_TAG_OBJECT_SHA="$(git rev-parse "$TAG_REF")"
REMOTE_TAG_COMMIT_SHA="$(git rev-parse "${TAG_REF}^{commit}")"

# A complete previous attempt may have pushed both refs but failed before the
# job recorded its output. Recognize that exact state and finish successfully.
if [[ "$REMOTE_MAIN_SHA" != "$PREPARE_SHA"       && "$REMOTE_TAG_COMMIT_SHA" == "$REMOTE_MAIN_SHA" ]]; then
  COMMITS_AHEAD="$(git rev-list --count "${PREPARE_SHA}..${REMOTE_MAIN_SHA}")"
  LAST_SUBJECT="$(git show -s --format=%s "$REMOTE_MAIN_SHA")"
  CHANGED_FILES="$(git diff --name-only "$PREPARE_SHA" "$REMOTE_MAIN_SHA")"

  if [[ "$COMMITS_AHEAD" == "1"         && "$LAST_SUBJECT" == "chore: update Flatpak Cargo sources [skip ci]"         && "$CHANGED_FILES" == "packaging/flatpak/cargo-sources.json" ]]; then
    echo "The generated Flatpak sources commit and release tag are already up to date."
    echo "sha=$REMOTE_MAIN_SHA" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

if [[ "$REMOTE_TAG_COMMIT_SHA" != "$PREPARE_SHA" ]]; then
  echo "::error::The release tag $RELEASE_TAG points to $REMOTE_TAG_COMMIT_SHA, not prepare's commit $PREPARE_SHA. Refusing to move it."
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# If main already contains exactly the generated-file commit, this is a retry
# after a partial push. Reuse that commit instead of creating a duplicate.
if [[ "$REMOTE_MAIN_SHA" == "$PREPARE_SHA" ]]; then
  git clone --depth 1 https://github.com/flatpak/flatpak-builder-tools "$TMP_DIR/flatpak-builder-tools"

  python3 -m venv "$TMP_DIR/venv"
  "$TMP_DIR/venv/bin/python" -m pip install --disable-pip-version-check aiohttp tomlkit

  "$TMP_DIR/venv/bin/python" \
    "$TMP_DIR/flatpak-builder-tools/cargo/flatpak-cargo-generator.py" \
    "$ROOT/Cargo.lock" \
    -o "$ROOT/packaging/flatpak/cargo-sources.json"

  if [[ ! -s "$ROOT/packaging/flatpak/cargo-sources.json" ]]; then
    echo "::error::The Flatpak cargo generator did not create a non-empty cargo-sources.json."
    exit 1
  fi

  git add -- packaging/flatpak/cargo-sources.json

  if git diff --cached --quiet; then
    echo "Flatpak cargo-sources.json is already up to date. No commit is needed."
    echo "sha=$PREPARE_SHA" >> "$GITHUB_OUTPUT"
    exit 0
  fi

  git config user.name "github-actions[bot]"
  git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  git commit -m "chore: update Flatpak Cargo sources [skip ci]"
  NEW_SHA="$(git rev-parse HEAD)"

  # The branch is expected to still be at the commit produced by prepare.
  # A normal (non-force) push refuses to overwrite any intervening main commit.
  MAIN_AUTH="$(printf 'x-access-token:%s' "$MAIN_PUSH_KEY" | base64 | tr -d '\n')"
  git -c "http.extraheader=AUTHORIZATION: basic ${MAIN_AUTH}" \
    push origin "${NEW_SHA}:refs/heads/main"
else
  # A retry may arrive after main was pushed but before the release tag moved.
  # Only accept the exact one-commit update made by this script.
  COMMITS_AHEAD="$(git rev-list --count "${PREPARE_SHA}..${REMOTE_MAIN_SHA}")"
  LAST_SUBJECT="$(git show -s --format=%s "$REMOTE_MAIN_SHA")"
  CHANGED_FILES="$(git diff --name-only "$PREPARE_SHA" "$REMOTE_MAIN_SHA")"

  if [[ "$COMMITS_AHEAD" != "1" \
        || "$LAST_SUBJECT" != "chore: update Flatpak Cargo sources [skip ci]" \
        || "$CHANGED_FILES" != "packaging/flatpak/cargo-sources.json" ]]; then
    echo "::error::main has moved beyond prepare's commit in an unexpected way. Refusing to overwrite it."
    exit 1
  fi

  NEW_SHA="$REMOTE_MAIN_SHA"
  echo "Found the generated Flatpak sources commit from an earlier attempt: $NEW_SHA"
fi

# Move the release tag to the commit that includes the generated JSON. This
# push intentionally uses GITHUB_TOKEN, not MAIN_PUSH_KEY: GitHub does not
# trigger another workflow run for events caused by this token.
GH_AUTH="$(printf 'x-access-token:%s' "$GITHUB_TOKEN" | base64 | tr -d '\n')"
git -c "http.extraheader=AUTHORIZATION: basic ${GH_AUTH}" push \
  --force-with-lease="${TAG_REF}:${REMOTE_TAG_OBJECT_SHA}" \
  origin "${NEW_SHA}:${TAG_REF}"

# Downstream build and deployment jobs must use the commit containing the JSON.
echo "sha=$NEW_SHA" >> "$GITHUB_OUTPUT"
echo "Flatpak cargo sources committed and release tag $RELEASE_TAG updated to $NEW_SHA."
