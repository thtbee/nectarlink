#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds a macOS .app bundle and compressed .dmg installer for NectarLink Desktop:
#   dist/Nectarlink-<version>-macos-<arch>.dmg

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DESKTOP_DIR="${REPO_ROOT}/desktop/app"

VERSION="${VERSION:-$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "${REPO_ROOT}/Cargo.toml")}"
if [[ -z "${VERSION}" ]]; then
    echo "error: could not determine version from ${REPO_ROOT}/Cargo.toml" >&2
    exit 1
fi
# CFBundleVersion requires numeric dot-separated components (strip -nightly.N suffix if present).
BUNDLE_VERSION="${VERSION%%-*}"
ARCH="${ARCH:-$(uname -m)}"
case "${ARCH}" in
    arm64|aarch64)
        RUST_TARGET="aarch64-apple-darwin"
        DMG_ARCH="arm64"
        ;;
    x86_64|amd64)
        RUST_TARGET="x86_64-apple-darwin"
        DMG_ARCH="x86_64"
        ;;
    *)
        echo "error: unsupported macOS architecture: ${ARCH}" >&2
        exit 1
        ;;
esac

if [[ -z "${MACDEPLOYQT:-}" ]]; then
    if [[ -n "${QT_ROOT_DIR:-}" && -x "${QT_ROOT_DIR}/bin/macdeployqt" ]]; then
        MACDEPLOYQT="${QT_ROOT_DIR}/bin/macdeployqt"
    elif command -v macdeployqt >/dev/null 2>&1; then
        MACDEPLOYQT="$(command -v macdeployqt)"
    else
        echo "error: macdeployqt not found; install Qt 6 or set QT_ROOT_DIR / MACDEPLOYQT" >&2
        exit 1
    fi
fi

BIN_PATH="${BIN_PATH:-}"
if [[ -z "${BIN_PATH}" ]]; then
    if [[ -f "${REPO_ROOT}/target/${RUST_TARGET}/release/nectarlink-desktop" ]]; then
        BIN_PATH="${REPO_ROOT}/target/${RUST_TARGET}/release/nectarlink-desktop"
    else
        BIN_PATH="${REPO_ROOT}/target/release/nectarlink-desktop"
    fi
fi

if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Building nectarlink-desktop (release, ${RUST_TARGET})..."
    (cd "${REPO_ROOT}" && cargo build --release --target "${RUST_TARGET}" -p nectarlink-desktop)
    BIN_PATH="${REPO_ROOT}/target/${RUST_TARGET}/release/nectarlink-desktop"
fi

STAGE_DIR="$(mktemp -d -t nectarlink-macos-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

APP_DIR="${STAGE_DIR}/NectarLink.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"
install -m 0755 "${BIN_PATH}" "${MACOS_DIR}/nectarlink-desktop"

# Translations go in Resources: Contents/MacOS is for code only, which
# is what codesign expects there.
qm_count=0
for qm in "$(dirname "${BIN_PATH}")"/nectarlink_*.qm; do
    if [[ -f "${qm}" ]]; then
        install -m 0644 "${qm}" "${RESOURCES_DIR}/$(basename "${qm}")"
        qm_count=$((qm_count + 1))
    fi
done
if [[ "${qm_count}" -eq 0 ]]; then
    echo "warning: no translations next to ${BIN_PATH}; the app will be English only" >&2
fi

sed -e "s/@VERSION@/${VERSION}/g" -e "s/@BUNDLE_VERSION@/${BUNDLE_VERSION}/g" \
    "${SCRIPT_DIR}/Info.plist.in" > "${CONTENTS_DIR}/Info.plist"

if [[ -f "${REPO_ROOT}/assets/brand/icon-512.png" ]]; then
    ICONSET_DIR="${STAGE_DIR}/NectarLink.iconset"
    mkdir -p "${ICONSET_DIR}"
    SRC_PNG="${REPO_ROOT}/assets/brand/icon-512.png"
    for size in 16 32 128 256; do
        sips -z "${size}" "${size}" "${SRC_PNG}" --out "${ICONSET_DIR}/icon_${size}x${size}.png" >/dev/null
        dbl=$((size * 2))
        sips -z "${dbl}" "${dbl}" "${SRC_PNG}" --out "${ICONSET_DIR}/icon_${size}x${size}@2x.png" >/dev/null
    done
    cp "${SRC_PNG}" "${ICONSET_DIR}/icon_512x512.png"
    iconutil -c icns "${ICONSET_DIR}" -o "${RESOURCES_DIR}/NectarLink.icns"
    rm -rf "${ICONSET_DIR}"
fi

echo "Bundling Qt frameworks via ${MACDEPLOYQT}..."
"${MACDEPLOYQT}" "${APP_DIR}" -qmldir="${DESKTOP_DIR}/qml" -always-overwrite

# Ad-hoc sign so Gatekeeper/macOS local launch doesn't fail on modified bundled frameworks.
codesign --force --deep --sign - "${APP_DIR}"

ln -s /Applications "${STAGE_DIR}/Applications"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
DMG_PATH="${OUT_DIR}/Nectarlink-${VERSION}-macos-${DMG_ARCH}.dmg"
rm -f "${DMG_PATH}"

echo "Creating ${DMG_PATH}..."
hdiutil create -volname "NectarLink ${VERSION}" \
    -srcfolder "${STAGE_DIR}" \
    -ov -format UDZO \
    "${DMG_PATH}" >/dev/null

echo "Built ${DMG_PATH}"
