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

STAGE_DIR="$(mktemp -d -t nectarlink-deb-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

PKG_ROOT="${STAGE_DIR}/nectarlink_${VERSION}_${DEB_ARCH}"
mkdir -p \
    "${PKG_ROOT}/DEBIAN" \
    "${PKG_ROOT}/usr/bin" \
    "${PKG_ROOT}/usr/lib/nectarlink" \
    "${PKG_ROOT}/usr/share/nectarlink" \
    "${PKG_ROOT}/usr/share/applications" \
    "${PKG_ROOT}/usr/share/metainfo" \
    "${PKG_ROOT}/usr/share/icons/hicolor/256x256/apps" \
    "${PKG_ROOT}/usr/share/icons/hicolor/512x512/apps" \
    "${PKG_ROOT}/usr/share/icons/hicolor/scalable/apps"

install -m 0755 "${BIN_PATH}" "${PKG_ROOT}/usr/lib/nectarlink/nectarlink-desktop"
for adapter in "$(dirname "${BIN_PATH}")"/libnectarlink-video-ffmpeg*.so; do
    [[ ! -f "${adapter}" ]] || install -m 0755 "${adapter}" "${PKG_ROOT}/usr/lib/nectarlink/"
done
cat > "${PKG_ROOT}/usr/bin/nectarlink-desktop" <<'EOF'
#!/usr/bin/env sh
if [ -d /usr/lib/nectarlink/lib ]; then
    export LD_LIBRARY_PATH="/usr/lib/nectarlink/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
    export QT_PLUGIN_PATH="/usr/lib/nectarlink/plugins${QT_PLUGIN_PATH:+:${QT_PLUGIN_PATH}}"
    export QML2_IMPORT_PATH="/usr/lib/nectarlink/qml${QML2_IMPORT_PATH:+:${QML2_IMPORT_PATH}}"
fi
exec /usr/lib/nectarlink/nectarlink-desktop "$@"
EOF
chmod 0755 "${PKG_ROOT}/usr/bin/nectarlink-desktop"

for qm in "$(dirname "${BIN_PATH}")"/nectarlink_*.qm; do
    if [[ -f "${qm}" ]]; then
        install -m 0644 "${qm}" "${PKG_ROOT}/usr/share/nectarlink/$(basename "${qm}")"
    fi
done

install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${PKG_ROOT}/usr/share/applications/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/org.nectarlink.desktop.metainfo.xml" "${PKG_ROOT}/usr/share/metainfo/org.nectarlink.desktop.metainfo.xml"

if [[ -f "${REPO_ROOT}/assets/brand/icon-512.png" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/brand/icon-512.png" "${PKG_ROOT}/usr/share/icons/hicolor/512x512/apps/nectarlink.png"
    install -m 0644 "${REPO_ROOT}/assets/brand/icon-512.png" "${PKG_ROOT}/usr/share/icons/hicolor/256x256/apps/nectarlink.png"
fi
if [[ -f "${REPO_ROOT}/assets/brand/icon.svg" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/brand/icon.svg" "${PKG_ROOT}/usr/share/icons/hicolor/scalable/apps/nectarlink.svg"
fi

QT_DEPENDS=""
bundle_private_qt_if_needed() {
    local qmake_bin="${QMAKE:-}"
    if [[ -z "${qmake_bin}" || ! -x "${qmake_bin}" ]]; then
        QT_DEPENDS=", libqt6core6t64 | libqt6core6, libqt6gui6, libqt6widgets6, libqt6qml6, libqt6quick6, libqt6network6, libqt6opengl6, libqt6dbus6, libqt6svg6, qml6-module-qtquick, qml6-module-qtquick-window, qml6-module-qtquick-controls, qml6-module-qtquick-layouts, qml6-module-qtqml-workerscript"
        return 0
    fi
    local qt_libs qt_plugins qt_qml
    qt_libs="$("${qmake_bin}" -query QT_INSTALL_LIBS 2>/dev/null || true)"
    qt_plugins="$("${qmake_bin}" -query QT_INSTALL_PLUGINS 2>/dev/null || true)"
    qt_qml="$("${qmake_bin}" -query QT_INSTALL_QML 2>/dev/null || true)"
    if [[ -z "${qt_libs}" || "${qt_libs}" == /usr/* ]]; then
        QT_DEPENDS=", libqt6core6t64 | libqt6core6, libqt6gui6, libqt6widgets6, libqt6qml6, libqt6quick6, libqt6network6, libqt6opengl6, libqt6dbus6, libqt6svg6, qml6-module-qtquick, qml6-module-qtquick-window, qml6-module-qtquick-controls, qml6-module-qtquick-layouts, qml6-module-qtqml-workerscript"
        return 0
    fi

    echo "Bundling private Qt runtime from ${qt_libs} into /usr/lib/nectarlink..."
    local dest="${PKG_ROOT}/usr/lib/nectarlink"
    mkdir -p "${dest}/lib" "${dest}/plugins" "${dest}/qml"

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
        "libicui18n.so*" \
        "libicuuc.so*" \
        "libicudata.so*"
    do
        compgen -G "${qt_libs}/${pattern}" >/dev/null && cp -a "${qt_libs}"/${pattern} "${dest}/lib/" || true
    done

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
                cp -a "${qt_plugins}/${plug_dir}" "${dest}/plugins/"
            fi
        done
    fi

    if [[ -n "${qt_qml}" && -d "${qt_qml}" ]]; then
        local qml_dir
        for qml_dir in QtQuick QtQml QtCore Qt; do
            if [[ -d "${qt_qml}/${qml_dir}" ]]; then
                cp -a "${qt_qml}/${qml_dir}" "${dest}/qml/"
            fi
        done
    fi

    cat > "${dest}/qt.conf" <<'QTEOF'
[Paths]
Prefix = .
Libraries = lib
Plugins = plugins
Qml2Imports = qml
QTEOF
}

bundle_private_qt_if_needed

INSTALLED_SIZE="$(du -sk "${PKG_ROOT}/usr" | awk '{print $1}')"

cat > "${PKG_ROOT}/DEBIAN/control" <<EOF
Package: nectarlink
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Installed-Size: ${INSTALLED_SIZE}
Depends: libc6, libgcc-s1, libstdc++6, libgl1, libx11-6, libxcb1, libxkbcommon0, libwayland-client0, libfontconfig1, libfreetype6, libdbus-1-3${QT_DEPENDS}
Recommends: fuse3, curl | wget, ffmpeg, gstreamer1.0-tools, gstreamer1.0-pipewire, gstreamer1.0-plugins-base, libopenh264-8 | libopenh264-7 | libopenh264-6 | libavcodec62 | libavcodec61 | libavcodec60, pipewire-bin | pulseaudio-utils, pipewire, wl-clipboard, xclip, xdg-utils, avahi-daemon, bluez, v4l2loopback-dkms, policykit-1 | pkexec
Maintainer: thtbee <https://github.com/alejandrokundrah/nectarlink>
Homepage: https://github.com/alejandrokundrah/nectarlink
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
