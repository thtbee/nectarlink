#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds NectarLinkCamera.systemextension and the optional activation helper.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="${1:-${SCRIPT_DIR}/build}"
EXT_ID="com.nectarlink.desktop.camera-extension"
EXT_BUNDLE="${OUT_DIR}/${EXT_ID}.systemextension"
EXT_CONTENTS="${EXT_BUNDLE}/Contents"
EXT_MACOS="${EXT_CONTENTS}/MacOS"

mkdir -p "${EXT_MACOS}"

swiftc \
    -O \
    -parse-as-library \
    -target "$(uname -m)-apple-macos12.3" \
    -framework CoreMedia \
    -framework CoreMediaIO \
    -framework CoreVideo \
    -framework Foundation \
    -framework IOKit \
    "${SCRIPT_DIR}/NectarLinkCamera/SharedFrameReader.swift" \
    "${SCRIPT_DIR}/NectarLinkCamera/CameraProvider.swift" \
    "${SCRIPT_DIR}/NectarLinkCamera/main.swift" \
    -o "${EXT_MACOS}/${EXT_ID}"

cp "${SCRIPT_DIR}/NectarLinkCamera/Info.plist" "${EXT_CONTENTS}/Info.plist"

if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp \
        --entitlements "${SCRIPT_DIR}/NectarLinkCamera/NectarLinkCamera.entitlements" \
        --sign "${CODESIGN_IDENTITY}" \
        "${EXT_BUNDLE}"
fi

swiftc \
    -O \
    -target "$(uname -m)-apple-macos12.3" \
    -framework Foundation \
    -framework SystemExtensions \
    "${SCRIPT_DIR}/ActivateExtension.swift" \
    -o "${OUT_DIR}/nectarlink-vcam-activate"

echo "Built ${EXT_BUNDLE}"
