#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds a Debian/Ubuntu .deb package for NectarLink Desktop:
#   dist/nectarlink_<version>_<arch>.deb

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DESKTOP_DIR="${REPO_ROOT}/desktop/app"

VERSION="${VERSION:-$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "${REPO_ROOT}/Cargo.toml")}"
if [[ -z "${VERSION}" ]]; then
    echo "error: could not determine version from ${REPO_ROOT}/Cargo.toml" >&2
    exit 1
fi

ARCH="${ARCH:-$(dpkg --print-architecture 2>/dev/null || uname -m)}"
case "${ARCH}" in
    x86_64|amd64) DEB_ARCH="amd64" ;;
    aarch64|arm64) DEB_ARCH="arm64" ;;
    *) DEB_ARCH="${ARCH}" ;;
esac

BIN_PATH="${BIN_PATH:-${REPO_ROOT}/target/release/nectarlink-desktop}"
if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Building nectarlink-desktop (release)..."
    (cd "${REPO_ROOT}" && cargo build --release -p nectarlink-desktop)
fi

STAGE_DIR="$(mktemp -d -t nectarlink-deb-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

PKG_ROOT="${STAGE_DIR}/nectarlink_${VERSION}_${DEB_ARCH}"
mkdir -p \
    "${PKG_ROOT}/DEBIAN" \
    "${PKG_ROOT}/usr/bin" \
    "${PKG_ROOT}/usr/share/applications" \
    "${PKG_ROOT}/usr/share/metainfo" \
    "${PKG_ROOT}/usr/share/icons/hicolor/256x256/apps" \
    "${PKG_ROOT}/usr/share/icons/hicolor/scalable/apps"

install -m 0755 "${BIN_PATH}" "${PKG_ROOT}/usr/bin/nectarlink-desktop"
for qm in "$(dirname "${BIN_PATH}")"/nectarlink_*.qm; do
    [[ -f "${qm}" ]] && install -m 0644 "${qm}" "${PKG_ROOT}/usr/bin/$(basename "${qm}")"
done
install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${PKG_ROOT}/usr/share/applications/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/org.nectarlink.desktop.metainfo.xml" "${PKG_ROOT}/usr/share/metainfo/org.nectarlink.desktop.metainfo.xml"

if [[ -f "${DESKTOP_DIR}/installer/nectarlink.png" ]]; then
    install -m 0644 "${DESKTOP_DIR}/installer/nectarlink.png" "${PKG_ROOT}/usr/share/icons/hicolor/256x256/apps/nectarlink.png"
fi

cat > "${PKG_ROOT}/usr/share/icons/hicolor/scalable/apps/nectarlink.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <rect width="256" height="256" rx="56" fill="#F59E0B"/>
  <path d="M128 44 L196 84 L196 172 L128 212 L60 172 L60 84 Z" fill="#2C1600"/>
  <circle cx="128" cy="128" r="36" fill="#FDE68A"/>
</svg>
EOF
chmod 0644 "${PKG_ROOT}/usr/share/icons/hicolor/scalable/apps/nectarlink.svg"

INSTALLED_SIZE="$(du -sk "${PKG_ROOT}/usr" | awk '{print $1}')"

cat > "${PKG_ROOT}/DEBIAN/control" <<EOF
Package: nectarlink
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Installed-Size: ${INSTALLED_SIZE}
Depends: libc6, libgcc-s1, libstdc++6, libgl1, libxkbcommon0, libwayland-client0, libfontconfig1, libfreetype6, libdbus-1-3, libpulse0
Recommends: avahi-daemon, bluez, ffmpeg, pipewire, wl-clipboard, xclip, sshfs, v4l2loopback-dkms, pkexec
Maintainer: NectarLink Contributors <noreply@nectarlink.org>
Homepage: https://github.com/thtbee/nectarlink
Description: Connect your Android phone and Linux PC over LAN
 NectarLink links your Android phone and Linux desktop over your local
 network with end-to-end encryption, clipboard sync, notification
 mirroring, file transfer, screen/camera/audio streaming, and SMS/calls.
EOF

cat > "${PKG_ROOT}/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
EOF
chmod 0755 "${PKG_ROOT}/DEBIAN/postinst"

cat > "${PKG_ROOT}/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
EOF
chmod 0755 "${PKG_ROOT}/DEBIAN/postrm"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
OUT_DEB="${OUT_DIR}/nectarlink_${VERSION}_${DEB_ARCH}.deb"

dpkg-deb --root-owner-group --build "${PKG_ROOT}" "${OUT_DEB}"
echo "Built ${OUT_DEB}"
