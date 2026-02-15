#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

ARCH="${ARCH:-x86_64}"
VERSION="${1:-$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)}"

DIST_DIR="${ROOT_DIR}/dist"
APPDIR="${DIST_DIR}/AppDir"
TOOLS_DIR="${DIST_DIR}/tools"
APPIMAGE_NAME="Archtoys-${VERSION}-${ARCH}.AppImage"
APPIMAGE_PATH="${DIST_DIR}/${APPIMAGE_NAME}"

mkdir -p "${DIST_DIR}" "${TOOLS_DIR}"
rm -rf "${APPDIR}"
mkdir -p \
  "${APPDIR}/usr/bin" \
  "${APPDIR}/usr/share/applications" \
  "${APPDIR}/usr/share/icons/hicolor/256x256/apps"

echo "[1/5] Building release binary..."
cargo build --release --locked

echo "[2/5] Preparing AppDir..."
install -Dm755 "${ROOT_DIR}/target/release/color-picker" "${APPDIR}/usr/bin/archtoys-bin"
cat > "${APPDIR}/usr/bin/archtoys" <<'SH'
#!/usr/bin/env sh
HERE="$(dirname "$(readlink -f "$0")")"
export SLINT_BACKEND="${SLINT_BACKEND:-winit}"
exec "${HERE}/archtoys-bin" "$@"
SH
chmod +x "${APPDIR}/usr/bin/archtoys"

install -Dm644 "${ROOT_DIR}/packaging/archtoys.desktop" "${APPDIR}/usr/share/applications/archtoys.desktop"
install -Dm644 "${ROOT_DIR}/packaging/archtoys-256.png" "${APPDIR}/usr/share/icons/hicolor/256x256/apps/archtoys.png"

cat > "${APPDIR}/AppRun" <<'SH'
#!/usr/bin/env sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/archtoys" "$@"
SH
chmod +x "${APPDIR}/AppRun"

cp "${APPDIR}/usr/share/applications/archtoys.desktop" "${APPDIR}/archtoys.desktop"
cp "${APPDIR}/usr/share/icons/hicolor/256x256/apps/archtoys.png" "${APPDIR}/archtoys.png"

LINUXDEPLOY="${TOOLS_DIR}/linuxdeploy-${ARCH}.AppImage"
APPIMAGETOOL="${TOOLS_DIR}/appimagetool-${ARCH}.AppImage"

if [[ ! -x "${LINUXDEPLOY}" ]]; then
  echo "[3/5] Downloading linuxdeploy..."
  curl -fL \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${ARCH}.AppImage" \
    -o "${LINUXDEPLOY}"
  chmod +x "${LINUXDEPLOY}"
fi

if [[ ! -x "${APPIMAGETOOL}" ]]; then
  echo "[3/5] Downloading appimagetool..."
  curl -fL \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage" \
    -o "${APPIMAGETOOL}"
  chmod +x "${APPIMAGETOOL}"
fi

echo "[4/5] Building AppImage..."
rm -f "${DIST_DIR}"/*.AppImage

set +e
(
  cd "${DIST_DIR}"
  APPIMAGE_EXTRACT_AND_RUN=1 ARCH="${ARCH}" \
    "${LINUXDEPLOY}" \
    --appdir "${APPDIR}" \
    -e "${APPDIR}/usr/bin/archtoys-bin" \
    -d "${APPDIR}/usr/share/applications/archtoys.desktop" \
    -i "${APPDIR}/usr/share/icons/hicolor/256x256/apps/archtoys.png" \
    --output appimage
)
LINUXDEPLOY_EXIT=$?
set -e

if [[ ${LINUXDEPLOY_EXIT} -ne 0 ]]; then
  echo "linuxdeploy failed, falling back to appimagetool..."
  APPIMAGE_EXTRACT_AND_RUN=1 ARCH="${ARCH}" \
    "${APPIMAGETOOL}" \
    "${APPDIR}" \
    "${APPIMAGE_PATH}"
else
  GENERATED="$(find "${DIST_DIR}" -maxdepth 1 -type f -name '*.AppImage' | head -n 1 || true)"
  if [[ -z "${GENERATED}" ]]; then
    echo "linuxdeploy did not produce an AppImage file." >&2
    exit 1
  fi
  mv -f "${GENERATED}" "${APPIMAGE_PATH}"
fi

echo "[5/5] Done."
echo "AppImage: ${APPIMAGE_PATH}"
