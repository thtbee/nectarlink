#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds a portable Linux tarball for NectarLink Desktop:
#   dist/Nectarlink-<version>-linux-<arch>.tar.gz
# including install.sh and uninstall.sh helper scripts.

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

STAGE_DIR="$(mktemp -d -t nectarlink-tar-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

BUNDLE_NAME="Nectarlink-${VERSION}-linux-${ARCH}"
BUNDLE_DIR="${STAGE_DIR}/${BUNDLE_NAME}"
mkdir -p "${BUNDLE_DIR}/share/applications" "${BUNDLE_DIR}/share/metainfo" "${BUNDLE_DIR}/share/icons"

install -m 0755 "${BIN_PATH}" "${BUNDLE_DIR}/nectarlink-desktop"
for qm in "$(dirname "${BIN_PATH}")"/nectarlink_*.qm; do
    [[ -f "${qm}" ]] && install -m 0644 "${qm}" "${BUNDLE_DIR}/$(basename "${qm}")"
done
install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${BUNDLE_DIR}/share/applications/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/org.nectarlink.desktop.metainfo.xml" "${BUNDLE_DIR}/share/metainfo/org.nectarlink.desktop.metainfo.xml"

cat > "${BUNDLE_DIR}/share/icons/nectarlink.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <rect width="256" height="256" rx="56" fill="#F59E0B"/>
  <path d="M128 44 L196 84 L196 172 L128 212 L60 172 L60 84 Z" fill="#2C1600"/>
  <circle cx="128" cy="128" r="36" fill="#FDE68A"/>
</svg>
EOF

cat > "${BUNDLE_DIR}/install.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    PREFIX="/opt/nectarlink"
    BIN_LINK="/usr/local/bin/nectarlink-desktop"
    APPS_DIR="/usr/share/applications"
    META_DIR="/usr/share/metainfo"
    ICON_DIR="/usr/share/icons/hicolor/scalable/apps"
else
    PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/nectarlink-app"
    BIN_LINK="${HOME}/.local/bin/nectarlink-desktop"
    APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    META_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/metainfo"
    ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
fi

mkdir -p "${PREFIX}" "$(dirname "${BIN_LINK}")" "${APPS_DIR}" "${META_DIR}" "${ICON_DIR}"
install -m 0755 "${SELF_DIR}/nectarlink-desktop" "${PREFIX}/nectarlink-desktop"
for qm in "${SELF_DIR}"/nectarlink_*.qm; do
    [[ -f "${qm}" ]] && install -m 0644 "${qm}" "${PREFIX}/$(basename "${qm}")"
done
ln -sf "${PREFIX}/nectarlink-desktop" "${BIN_LINK}"
install -m 0644 "${SELF_DIR}/share/applications/nectarlink.desktop" "${APPS_DIR}/nectarlink.desktop"
install -m 0644 "${SELF_DIR}/share/metainfo/org.nectarlink.desktop.metainfo.xml" "${META_DIR}/org.nectarlink.desktop.metainfo.xml"
install -m 0644 "${SELF_DIR}/share/icons/nectarlink.svg" "${ICON_DIR}/nectarlink.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "${APPS_DIR}" || true
fi
echo "Installed NectarLink to ${PREFIX} (${BIN_LINK})"
EOF
chmod 0755 "${BUNDLE_DIR}/install.sh"

cat > "${BUNDLE_DIR}/uninstall.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    rm -rf /opt/nectarlink
    rm -f /usr/local/bin/nectarlink-desktop
    rm -f /usr/share/applications/nectarlink.desktop
    rm -f /usr/share/metainfo/org.nectarlink.desktop.metainfo.xml
    rm -f /usr/share/icons/hicolor/scalable/apps/nectarlink.svg
else
    PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/nectarlink-app"
    rm -rf "${PREFIX}"
    rm -f "${HOME}/.local/bin/nectarlink-desktop"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/applications/nectarlink.desktop"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/metainfo/org.nectarlink.desktop.metainfo.xml"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/nectarlink.svg"
fi
echo "Uninstalled NectarLink."
EOF
chmod 0755 "${BUNDLE_DIR}/uninstall.sh"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
OUT_TAR="${OUT_DIR}/${BUNDLE_NAME}.tar.gz"

tar -C "${STAGE_DIR}" -czf "${OUT_TAR}" "${BUNDLE_NAME}"
echo "Built ${OUT_TAR}"
