#!/usr/bin/env bash
# Text Extractor's "Turn on", without the password dialog: runs the exact
# install script the app runs (.github/scripts/tesseract-install/<distro>.sh,
# kept equal to the app's by a unit test) as root in a clean container, then
# checks Tesseract works for Archtoys: version 4.1 or newer, English and the
# script detector (osd) installed.
#
#   bash .github/scripts/tesseract-install-test.sh archlinux:latest arch
#   bash .github/scripts/tesseract-install-test.sh fedora:latest fedora
#   bash .github/scripts/tesseract-install-test.sh ubuntu:22.04 debian
#
# With "preinstalled" as a third argument, an older Tesseract from the same
# distro is installed first: Turn on must still work (it reinstalls/updates).
set -euo pipefail
image=$1
distro=$2
mode=${3:-fresh}
here=$(cd "$(dirname "$0")" && pwd)

# a real system has its package lists; a fresh container doesn't
prepare='true'
case "$distro" in
  arch) prepare='pacman -Sy --noconfirm >/dev/null' ;;
  debian) prepare='apt-get update -qq >/dev/null' ;;
esac
preinstall='true'
if [ "$mode" = preinstalled ]; then
  case "$distro" in
    arch) preinstall='pacman -S --noconfirm tesseract tesseract-data-eng >/dev/null' ;;
    fedora) preinstall='dnf install -y tesseract >/dev/null' ;;
    debian) preinstall='DEBIAN_FRONTEND=noninteractive apt-get install -y -qq tesseract-ocr >/dev/null' ;;
  esac
fi

docker run --rm -v "$here/tesseract-install:/scripts:ro" "$image" sh -c "
  set -e
  $prepare
  $preinstall
  sh /scripts/$distro.sh > /tmp/install.log 2>&1 || { tail -n 30 /tmp/install.log; exit 1; }
  version=\$(tesseract --version 2>&1 | head -n 1)
  echo \"installed: \$version\"
  v=\$(echo \"\$version\" | grep -oE '[0-9]+\.[0-9]+' | head -n 1)
  major=\${v%%.*}; minor=\${v#*.}
  if [ \"\$major\" -lt 4 ] || { [ \"\$major\" -eq 4 ] && [ \"\$minor\" -lt 1 ]; }; then
    echo \"too old: \$v (Archtoys needs 4.1)\"; exit 1
  fi
  langs=\$(tesseract --list-langs 2>&1)
  echo \"\$langs\" | grep -qx eng || { echo 'English is missing'; echo \"\$langs\"; exit 1; }
  echo \"\$langs\" | grep -qx osd || { echo 'the script detector (osd) is missing'; echo \"\$langs\"; exit 1; }
  echo 'ok: version, English and osd'
"
