#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds an AppDir layout and invokes appimagetool (if present in PATH) to
# produce dist/Nectarlink-<version>-<arch>.AppImage.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DESKTOP_DIR="${REPO_ROOT}/desktop/app"

VERSION="${VERSION:-$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "${REPO_ROOT}/Cargo.toml")}"
if [[ -z "${VERSION}" ]]; then
    echo "error: could not determine version from ${REPO_ROOT}/Cargo.toml" >&2
    exit 1
fi

ARCH="${ARCH:-$(uname -m)}"
BIN_PATH="${BIN_PATH:-${REPO_ROOT}/target/release/nectarlink-desktop}"
if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Building nectarlink-desktop (release)..."
    (cd "${REPO_ROOT}" && cargo build --release -p nectarlink-desktop)
fi

STAGE_DIR="$(mktemp -d -t nectarlink-appimage-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

APPDIR="${STAGE_DIR}/NectarLink.AppDir"
mkdir -p \
    "${APPDIR}/usr/bin" \
    "${APPDIR}/usr/share/applications" \
    "${APPDIR}/usr/share/metainfo" \
    "${APPDIR}/usr/share/icons/hicolor/scalable/apps"

install -m 0755 "${BIN_PATH}" "${APPDIR}/usr/bin/nectarlink-desktop"
for qm in "$(dirname "${BIN_PATH}")"/nectarlink_*.qm; do
    [[ -f "${qm}" ]] && install -m 0644 "${qm}" "${APPDIR}/usr/bin/$(basename "${qm}")"
done
install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${APPDIR}/usr/share/applications/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${APPDIR}/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/org.nectarlink.desktop.metainfo.xml" "${APPDIR}/usr/share/metainfo/org.nectarlink.desktop.metainfo.xml"

cat > "${APPDIR}/usr/share/icons/hicolor/scalable/apps/nectarlink.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <rect width="256" height="256" rx="56" fill="#F59E0B"/>
  <path d="M128 44 L196 84 L196 172 L128 212 L60 172 L60 84 Z" fill="#2C1600"/>
  <circle cx="128" cy="128" r="36" fill="#FDE68A"/>
</svg>
EOF
cp "${APPDIR}/usr/share/icons/hicolor/scalable/apps/nectarlink.svg" "${APPDIR}/nectarlink.svg"

cat > "${APPDIR}/AppRun" <<'EOF'
#!/usr/bin/env sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/nectarlink-desktop" "$@"
EOF
chmod 0755 "${APPDIR}/AppRun"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
OUT_APPIMAGE="${OUT_DIR}/Nectarlink-${VERSION}-${ARCH}.AppImage"

if command -v appimagetool >/dev/null 2>&1; then
    ARCH="${ARCH}" appimagetool "${APPDIR}" "${OUT_APPIMAGE}"
    echo "Built ${OUT_APPIMAGE}"
else
    echo "appimagetool not found in PATH; AppDir staged at ${APPDIR}" >&2
    exit 0
fi
