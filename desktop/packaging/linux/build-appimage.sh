#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds an AppDir layout (including bundled Qt 6 runtime libraries, plugins,
# and QML modules) and invokes appimagetool (if present in PATH) to produce
# dist/Nectarlink-<version>-<arch>.AppImage.

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

# Locate Qt 6 qmake if available so we can bundle the matching Qt 6.12+ runtime.
if [[ -z "${QMAKE:-}" ]]; then
    if [[ -n "${QT_ROOT_DIR:-}" && -x "${QT_ROOT_DIR}/bin/qmake" ]]; then
        export QMAKE="${QT_ROOT_DIR}/bin/qmake"
    elif [[ -x "${REPO_ROOT}/.qt6-sysroot/6.12.0/gcc_64/bin/qmake" ]]; then
        export QMAKE="${REPO_ROOT}/.qt6-sysroot/6.12.0/gcc_64/bin/qmake"
    elif command -v qmake6 >/dev/null 2>&1; then
        export QMAKE="$(command -v qmake6)"
    elif command -v qmake >/dev/null 2>&1; then
        export QMAKE="$(command -v qmake)"
    fi
fi

BIN_PATH="${BIN_PATH:-${REPO_ROOT}/target/release/nectarlink-desktop}"
if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Building nectarlink-desktop (release)..."
    (cd "${REPO_ROOT}" && cargo build --release -p nectarlink-desktop)
fi

STAGE_DIR="$(mktemp -d -t nectarlink-appimage-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

bash "${SCRIPT_DIR}/build-translations.sh" "${STAGE_DIR}/translations"

APPDIR="${STAGE_DIR}/NectarLink.AppDir"
mkdir -p \
    "${APPDIR}/usr/bin" \
    "${APPDIR}/usr/lib" \
    "${APPDIR}/usr/share/nectarlink" \
    "${APPDIR}/usr/share/applications" \
    "${APPDIR}/usr/share/metainfo" \
    "${APPDIR}/usr/share/icons/hicolor/256x256/apps" \
    "${APPDIR}/usr/share/icons/hicolor/512x512/apps" \
    "${APPDIR}/usr/share/icons/hicolor/scalable/apps"

install -m 0755 "${BIN_PATH}" "${APPDIR}/usr/bin/nectarlink-desktop"
for adapter in "$(dirname "${BIN_PATH}")"/libnectarlink-video-ffmpeg*.so; do
    [[ ! -f "${adapter}" ]] || install -m 0755 "${adapter}" "${APPDIR}/usr/bin/"
done

for qm in "${STAGE_DIR}"/translations/nectarlink_*.qm; do
    if [[ -f "${qm}" ]]; then
        install -m 0644 "${qm}" "${APPDIR}/usr/share/nectarlink/$(basename "${qm}")"
    fi
done

install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${APPDIR}/usr/share/applications/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/nectarlink.desktop" "${APPDIR}/nectarlink.desktop"
install -m 0644 "${SCRIPT_DIR}/org.nectarlink.desktop.metainfo.xml" "${APPDIR}/usr/share/metainfo/org.nectarlink.desktop.metainfo.xml"

if [[ -f "${REPO_ROOT}/assets/brand/icon.svg" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/brand/icon.svg" "${APPDIR}/usr/share/icons/hicolor/scalable/apps/nectarlink.svg"
    install -m 0644 "${REPO_ROOT}/assets/brand/icon.svg" "${APPDIR}/nectarlink.svg"
fi
if [[ -f "${REPO_ROOT}/assets/brand/icon-512.png" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/brand/icon-512.png" "${APPDIR}/usr/share/icons/hicolor/512x512/apps/nectarlink.png"
    install -m 0644 "${REPO_ROOT}/assets/brand/icon-512.png" "${APPDIR}/usr/share/icons/hicolor/256x256/apps/nectarlink.png"
    install -m 0644 "${REPO_ROOT}/assets/brand/icon-512.png" "${APPDIR}/nectarlink.png"
fi

bundle_qt_runtime() {
    local qmake_bin="${QMAKE:-}"
    if [[ -z "${qmake_bin}" || ! -x "${qmake_bin}" ]]; then
        return 0
    fi
    local qt_libs qt_plugins qt_qml
    qt_libs="$("${qmake_bin}" -query QT_INSTALL_LIBS 2>/dev/null || true)"
    qt_plugins="$("${qmake_bin}" -query QT_INSTALL_PLUGINS 2>/dev/null || true)"
    qt_qml="$("${qmake_bin}" -query QT_INSTALL_QML 2>/dev/null || true)"
    if [[ -z "${qt_libs}" || ! -d "${qt_libs}" ]]; then
        return 0
    fi

    echo "Bundling Qt runtime from ${qt_libs}..."
    mkdir -p "${APPDIR}/usr/lib" "${APPDIR}/usr/plugins" "${APPDIR}/usr/qml"

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
        compgen -G "${qt_libs}/${pattern}" >/dev/null && cp -a "${qt_libs}"/${pattern} "${APPDIR}/usr/lib/" || true
    done
    # Caught by the patterns above but unused, and they need Qt modules that
    # aren't bundled (Sql, Test).
    rm -f "${APPDIR}/usr/lib"/libQt6QmlLocalStorage.so* "${APPDIR}/usr/lib"/libQt6QuickTest.so*

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
                cp -a "${qt_plugins}/${plug_dir}" "${APPDIR}/usr/plugins/"
            fi
        done
    fi

    if [[ -n "${qt_qml}" && -d "${qt_qml}" ]]; then
        local qml_dir
        for qml_dir in QtQuick QtQml QtCore Qt; do
            if [[ -d "${qt_qml}/${qml_dir}" ]]; then
                cp -a "${qt_qml}/${qml_dir}" "${APPDIR}/usr/qml/"
            fi
        done
    fi

    cat > "${APPDIR}/usr/bin/qt.conf" <<'QTEOF'
[Paths]
Prefix = ..
Libraries = lib
Plugins = plugins
Qml2Imports = qml
QTEOF
}

bundle_qt_runtime

cat > "${APPDIR}/AppRun" <<'EOF'
#!/usr/bin/env sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
export QT_PLUGIN_PATH="${HERE}/usr/plugins${QT_PLUGIN_PATH:+:${QT_PLUGIN_PATH}}"
export QML2_IMPORT_PATH="${HERE}/usr/qml${QML2_IMPORT_PATH:+:${QML2_IMPORT_PATH}}"
exec "${HERE}/usr/bin/nectarlink-desktop" "$@"
EOF
chmod 0755 "${APPDIR}/AppRun"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
OUT_APPIMAGE="${OUT_DIR}/Nectarlink-${VERSION}-${ARCH}.AppImage"

if ! command -v appimagetool >/dev/null 2>&1; then
    echo "appimagetool not found in PATH; AppDir staged at ${APPDIR}" >&2
    exit 1
fi
# APPIMAGE_RUNTIME: a runtime file checked beforehand (the release workflow
# pins one). Without it appimagetool downloads the latest runtime itself.
runtime_args=()
if [[ -n "${APPIMAGE_RUNTIME:-}" ]]; then
    runtime_args=(--runtime-file "${APPIMAGE_RUNTIME}")
fi
ARCH="${ARCH}" appimagetool "${runtime_args[@]}" "${APPDIR}" "${OUT_APPIMAGE}"
echo "Built ${OUT_APPIMAGE}"
