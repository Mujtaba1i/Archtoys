#!/usr/bin/env bash
# Pushes to main from a workflow (the "Release vX" commit, "Add supporter").
#
#   MAIN_PUSH_KEY=... bash .github/scripts/push-main.sh HEAD:refs/heads/main
#
# The "Require CI" rule on main lets a commit in only after CI passed on it,
# and a workflow's own new commit hasn't had CI yet. GitHub doesn't let a
# personal repository's rule wave through the GitHub Actions bot, but it can
# wave through a deploy key: MAIN_PUSH_KEY is the private half of a deploy key
# with write access that the rule's bypass list allows. Without it, this is a
# plain push with the workflow's token (enough when main has no such rule).
set -euo pipefail
if [ -z "${MAIN_PUSH_KEY:-}" ]; then
  exec git push --quiet origin "$@"
fi
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
printf '%s\n' "$MAIN_PUSH_KEY" > "$dir/key"
chmod 600 "$dir/key"
# GitHub's published SSH host key, so the push can't go to anyone else
echo "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl" > "$dir/known_hosts"
GIT_SSH_COMMAND="ssh -i $dir/key -o IdentitiesOnly=yes -o UserKnownHostsFile=$dir/known_hosts -o StrictHostKeyChecking=yes" \
  git push --quiet "git@github.com:${GITHUB_REPOSITORY:?}.git" "$@"
