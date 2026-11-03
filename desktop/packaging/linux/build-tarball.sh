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
# Locate Qt 6 qmake if available so we can bundle a private Qt 6.12+ runtime
# when built against a non-system Qt SDK (e.g. aqtinstall on Ubuntu 24.04).
if [[ -z "${QMAKE:-}" ]]; then
    if [[ -n "${QT_ROOT_DIR:-}" && -x "${QT_ROOT_DIR}/bin/qmake" ]]; then
        export QMAKE="${QT_ROOT_DIR}/bin/qmake"
    elif [[ -x "${REPO_ROOT}/.qt6-sysroot/6.12.0/gcc_64/bin/qmake" ]]; then
        export QMAKE="${REPO_ROOT}/.qt6-sysroot/6.12.0/gcc_64/bin/qmake"
    fi
fi

BIN_PATH="${BIN_PATH:-${REPO_ROOT}/target/release/nectarlink-desktop}"
if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Building nectarlink-desktop (release)..."
    (cd "${REPO_ROOT}" && cargo build --release -p nectarlink-desktop)
fi

STAGE_DIR="$(mktemp -d -t nectarlink-tar-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

BUNDLE_NAME="Nectarlink-${VERSION}-linux-${ARCH}"
BUNDLE_DIR="${STAGE_DIR}/${BUNDLE_NAME}"
mkdir -p "${BUNDLE_DIR}/share/applications" "${BUNDLE_DIR}/share/metainfo" "${BUNDLE_DIR}/share/icons" "${BUNDLE_DIR}/share/nectarlink"

install -m 0755 "${BIN_PATH}" "${BUNDLE_DIR}/nectarlink-desktop"
for adapter in "$(dirname "${BIN_PATH}")"/libnectarlink-video-ffmpeg*.so; do
    [[ ! -f "${adapter}" ]] || install -m 0755 "${adapter}" "${BUNDLE_DIR}/"
done
printf '%s\n' "${VERSION}" > "${BUNDLE_DIR}/VERSION"
for qm in "$(dirname "${BIN_PATH}")"/nectarlink_*.qm; do
    if [[ -f "${qm}" ]]; then
        install -m 0644 "${qm}" "${BUNDLE_DIR}/$(basename "${qm}")"
        install -m 0644 "${qm}" "${BUNDLE_DIR}/share/nectarlink/$(basename "${qm}")"
    fi
done
install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${BUNDLE_DIR}/share/applications/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/org.nectarlink.desktop.metainfo.xml" "${BUNDLE_DIR}/share/metainfo/org.nectarlink.desktop.metainfo.xml"

if [[ -f "${REPO_ROOT}/assets/brand/icon.svg" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/brand/icon.svg" "${BUNDLE_DIR}/share/icons/nectarlink.svg"
fi
if [[ -f "${REPO_ROOT}/assets/brand/icon-512.png" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/brand/icon-512.png" "${BUNDLE_DIR}/share/icons/nectarlink.png"
fi

bundle_private_qt_if_needed() {
    local qmake_bin="${QMAKE:-}"
    if [[ -z "${qmake_bin}" || ! -x "${qmake_bin}" ]]; then
        return 0
    fi
    local qt_libs qt_plugins qt_qml
    qt_libs="$("${qmake_bin}" -query QT_INSTALL_LIBS 2>/dev/null || true)"
    qt_plugins="$("${qmake_bin}" -query QT_INSTALL_PLUGINS 2>/dev/null || true)"
    qt_qml="$("${qmake_bin}" -query QT_INSTALL_QML 2>/dev/null || true)"
    if [[ -z "${qt_libs}" || "${qt_libs}" == /usr/* ]]; then
        return 0
    fi

    echo "Bundling private Qt runtime from ${qt_libs} into tarball..."
    mkdir -p "${BUNDLE_DIR}/lib" "${BUNDLE_DIR}/plugins" "${BUNDLE_DIR}/qml"

    local pattern
    for pattern in \
        "libQt6Core.so*" \
        "libQt6Gui.so*" \
        "libQt6Widgets.so*" \
        "libQt6Qml*.so*" \
        "libQt6Quick*.so*" \
        "libQt6Network.so*" \
        "libQt6OpenGL.so*" \
        "libQt6Svg.so*" \
        "libQt6DBus.so*" \
        "libQt6XcbQpa.so*" \
        "libQt6WaylandClient.so*" \
        "libQt6WaylandEglClientHwIntegration.so*" \
        "libQt6WlShellIntegration.so*" \
        "libQt6ShaderTools.so*" \
        "libQt6LabsFolderListModel.so*" \
        "libicui18n.so*" \
        "libicuuc.so*" \
        "libicudata.so*"
    do
        compgen -G "${qt_libs}/${pattern}" >/dev/null && cp -a "${qt_libs}"/${pattern} "${BUNDLE_DIR}/lib/" || true
    done
    # Caught by the patterns above but unused, and they need Qt modules that
    # aren't bundled (Sql, Test).
    rm -f "${BUNDLE_DIR}/lib"/libQt6QmlLocalStorage.so* "${BUNDLE_DIR}/lib"/libQt6QuickTest.so*

    if [[ -n "${qt_plugins}" && -d "${qt_plugins}" ]]; then
        local plug_dir
        for plug_dir in \
            platforms \
            platformthemes \
            wayland-decoration-client \
            wayland-graphics-integration-client \
            wayland-shell-integration \
            xcbglintegrations \
            imageformats \
            iconengines \
            tls
        do
            if [[ -d "${qt_plugins}/${plug_dir}" ]]; then
                cp -a "${qt_plugins}/${plug_dir}" "${BUNDLE_DIR}/plugins/"
            fi
        done
    fi

    if [[ -n "${qt_qml}" && -d "${qt_qml}" ]]; then
        local qml_dir
        for qml_dir in QtQuick QtQml QtCore Qt; do
            if [[ -d "${qt_qml}/${qml_dir}" ]]; then
                cp -a "${qt_qml}/${qml_dir}" "${BUNDLE_DIR}/qml/"
            fi
        done
    fi

    cat > "${BUNDLE_DIR}/qt.conf" <<'QTEOF'
[Paths]
Prefix = .
Libraries = lib
Plugins = plugins
Qml2Imports = qml
QTEOF
}

bundle_private_qt_if_needed

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
    PNG_ICON_DIR="/usr/share/icons/hicolor/512x512/apps"
else
    PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/nectarlink-app"
    BIN_LINK="${HOME}/.local/bin/nectarlink-desktop"
    APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    META_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/metainfo"
    ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
    PNG_ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps"
fi

mkdir -p "${PREFIX}" "$(dirname "${BIN_LINK}")" "${APPS_DIR}" "${META_DIR}" "${ICON_DIR}" "${PNG_ICON_DIR}"
install -m 0755 "${SELF_DIR}/nectarlink-desktop" "${PREFIX}/nectarlink-desktop"
for adapter in "${SELF_DIR}"/libnectarlink-video-ffmpeg*.so; do
    [[ ! -f "${adapter}" ]] || install -m 0755 "${adapter}" "${PREFIX}/"
done
[[ -f "${SELF_DIR}/VERSION" ]] && install -m 0644 "${SELF_DIR}/VERSION" "${PREFIX}/VERSION"
[[ -f "${SELF_DIR}/qt.conf" ]] && install -m 0644 "${SELF_DIR}/qt.conf" "${PREFIX}/qt.conf"
for dir in lib plugins qml; do
    if [[ -d "${SELF_DIR}/${dir}" ]]; then
        rm -rf "${PREFIX:?}/${dir}"
        cp -a "${SELF_DIR}/${dir}" "${PREFIX}/${dir}"
    fi
done
for qm in "${SELF_DIR}"/nectarlink_*.qm; do
    [[ -f "${qm}" ]] && install -m 0644 "${qm}" "${PREFIX}/$(basename "${qm}")"
done
if [[ -d "${PREFIX}/lib" ]]; then
    # An earlier install may have left a symlink here; writing through it
    # would overwrite the real binary with this wrapper.
    rm -f "${BIN_LINK}"
    cat > "${BIN_LINK}" <<WRAPEOF
#!/usr/bin/env sh
export LD_LIBRARY_PATH="${PREFIX}/lib\${LD_LIBRARY_PATH:+:\${LD_LIBRARY_PATH}}"
export QT_PLUGIN_PATH="${PREFIX}/plugins\${QT_PLUGIN_PATH:+:\${QT_PLUGIN_PATH}}"
export QML2_IMPORT_PATH="${PREFIX}/qml\${QML2_IMPORT_PATH:+:\${QML2_IMPORT_PATH}}"
exec "${PREFIX}/nectarlink-desktop" "\$@"
WRAPEOF
    chmod 0755 "${BIN_LINK}"
else
    ln -sf "${PREFIX}/nectarlink-desktop" "${BIN_LINK}"
fi
install -m 0644 "${SELF_DIR}/share/applications/nectarlink.desktop" "${APPS_DIR}/nectarlink.desktop"
install -m 0644 "${SELF_DIR}/share/metainfo/org.nectarlink.desktop.metainfo.xml" "${META_DIR}/org.nectarlink.desktop.metainfo.xml"
[[ -f "${SELF_DIR}/share/icons/nectarlink.svg" ]] && install -m 0644 "${SELF_DIR}/share/icons/nectarlink.svg" "${ICON_DIR}/nectarlink.svg"
[[ -f "${SELF_DIR}/share/icons/nectarlink.png" ]] && install -m 0644 "${SELF_DIR}/share/icons/nectarlink.png" "${PNG_ICON_DIR}/nectarlink.png"

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
    rm -f /usr/share/icons/hicolor/512x512/apps/nectarlink.png
else
    PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/nectarlink-app"
    rm -rf "${PREFIX}"
    rm -f "${HOME}/.local/bin/nectarlink-desktop"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/applications/nectarlink.desktop"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/metainfo/org.nectarlink.desktop.metainfo.xml"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/nectarlink.svg"
    rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps/nectarlink.png"
fi
echo "Uninstalled NectarLink."
EOF
chmod 0755 "${BUNDLE_DIR}/uninstall.sh"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
OUT_TAR="${OUT_DIR}/${BUNDLE_NAME}.tar.gz"

tar -C "${STAGE_DIR}" -czf "${OUT_TAR}" "${BUNDLE_NAME}"
echo "Built ${OUT_TAR}"
