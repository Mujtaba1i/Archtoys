#!/usr/bin/env bash
# Checks a built .deb before it is tested and published: the dependencies the
# app needs at run time, and the files it must install. Used by the optional
# "Build + test the .deb" job of release.yml.
#
# The libraries below are the ones the app loads while it runs (X11, keyboard,
# cursor, input, graphics). cargo-deb finds the ones the binary links to by
# itself ($auto), but not these, so they are listed under depends in
# [package.metadata.deb] in Cargo.toml. Keep both lists in step.
set -euo pipefail

DEB="${1:?Usage: check-deb.sh file.deb}"
fail() { echo "::error title=The .deb is not right::$*"; exit 1; }

[ -f "$DEB" ] || fail "$DEB does not exist"
depends="$(dpkg-deb -f "$DEB" Depends)"
echo "Depends: $depends"

for pkg in libc6 libfontconfig1 libx11-6 libxkbcommon0 libxkbcommon-x11-0 libxcursor1 libxi6 libxrender1 libgl1 libegl1; do
  grep -qE "(^|[ ,])$pkg([ ,(]|$)" <<<"$depends" \
    || fail "the package does not depend on $pkg. Add it to depends in [package.metadata.deb] in Cargo.toml."
done

files="$(dpkg-deb -c "$DEB")"
for path in \
  ./usr/bin/archtoys \
  ./usr/share/applications/archtoys.desktop \
  ./usr/share/icons/hicolor/16x16/apps/archtoys.png \
  ./usr/share/icons/hicolor/256x256/apps/archtoys.png \
  ./usr/share/icons/hicolor/1024x1024/apps/archtoys.png \
  ./usr/share/doc/archtoys/copyright \
  ./usr/share/doc/archtoys/colornames-LICENSE.txt
do
  grep -qF " $path" <<<"$files" \
    || fail "the package does not install $path. Check assets in [package.metadata.deb] in Cargo.toml."
done

name="$(dpkg-deb -f "$DEB" Package)"
[ "$name" = "archtoys" ] || fail "the package is called '$name', not archtoys."
echo "Package: $name $(dpkg-deb -f "$DEB" Version), $(grep -c . <<<"$files") entries. OK"
