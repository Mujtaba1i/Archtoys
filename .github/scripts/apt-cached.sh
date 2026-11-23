#!/usr/bin/env bash
# sudo apt-get install, keeping the downloaded packages between CI runs: the
# job's actions/cache step restores ~/apt-cache before this and saves it after
# the job, so Ubuntu's servers (sometimes very slow) only serve what changed.
#
#   bash .github/scripts/apt-cached.sh xvfb xdotool ...
set -euo pipefail
cache="$HOME/apt-cache"
mkdir -p "$cache"
shopt -s nullglob
kept=("$cache"/*.deb)
echo "${#kept[@]} packages from the cache"
[ "${#kept[@]}" = 0 ] || sudo cp -n "${kept[@]}" /var/cache/apt/archives/
sudo apt-get update
sudo apt-get install -y -o APT::Keep-Downloaded-Packages=true "$@"
# drop versions Ubuntu no longer has, then keep exactly what apt now holds
sudo apt-get autoclean -qq
rm -f "$cache"/*.deb
cp /var/cache/apt/archives/*.deb "$cache"/
