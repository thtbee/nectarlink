#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the macOS virtual camera extension into OUT_DIR (default ./build):
#   app.nectarlink.desktop.camera-extension.systemextension   the CoreMediaIO extension
#   NectarLinkCamera.entitlements, HostApp.entitlements        rendered for TEAM_ID
# The app installs it itself (desktop/app/cpp/mac_sysext.mm).
# desktop/packaging/macos/build-dmg.sh runs this and embeds the results in
# NectarLink.app; see README.md for the signing and profile setup.
#
#   ARCHS               "arm64", "x86_64" or "universal" (default: this Mac's)
#   VERSION             extension version (default: 0.0.0)
#   TEAM_ID             Apple team ID; the App Group becomes <TEAM_ID>.group.com.nectarlink.desktop
#                       (without it the bare group.com.nectarlink.desktop is used)
#   PROFILE_KEYS=1      add com.apple.application-identifier and the team identifier to
#                       the host app's entitlements, which a provisioning-profile-backed
#                       signature needs
#   EXT_PROFILE         the extension's .provisionprofile, copied into the bundle. Optional:
#                       the extension's entitlements (sandbox, team-prefixed App Group) aren't
#                       restricted, so it works without one. When set, its entitlements get
#                       the identity keys too.
#   HOST_BUNDLE_ID      the app's bundle ID (default: app.nectarlink.desktop)
#   CODESIGN_IDENTITY   sign the extension (hardened runtime, timestamp);
#                       leave it unset when build-dmg.sh signs them
#   KEYCHAIN_PATH       keychain holding CODESIGN_IDENTITY
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="${1:-${SCRIPT_DIR}/build}"
HOST_BUNDLE_ID="${HOST_BUNDLE_ID:-app.nectarlink.desktop}"
EXT_ID="${HOST_BUNDLE_ID}.camera-extension"
EXT_BUNDLE="${OUT_DIR}/${EXT_ID}.systemextension"
EXT_CONTENTS="${EXT_BUNDLE}/Contents"
EXT_MACOS="${EXT_CONTENTS}/MacOS"
VERSION="${VERSION:-0.0.0}"
BUNDLE_VERSION="${BUNDLE_VERSION:-${VERSION%%-*}}"
TEAM_ID="${TEAM_ID:-}"
GROUP_ID="${TEAM_ID:+${TEAM_ID}.}group.com.nectarlink.desktop"
PLISTBUDDY=/usr/libexec/PlistBuddy

case "${ARCHS:-$(uname -m)}" in
    universal) ARCH_LIST=(arm64 x86_64) ;;
    arm64 | aarch64) ARCH_LIST=(arm64) ;;
    x86_64 | amd64) ARCH_LIST=(x86_64) ;;
    "arm64 x86_64" | "x86_64 arm64") ARCH_LIST=(arm64 x86_64) ;;
    *)
        echo "error: unsupported ARCHS '${ARCHS}'" >&2
        exit 1
        ;;
esac

rm -rf "${EXT_BUNDLE}"
mkdir -p "${EXT_MACOS}"
WORK_DIR="$(mktemp -d -t nectarlink-vcam-XXXXXX)"
trap 'rm -rf "${WORK_DIR}"' EXIT

# swift_build <output> <min macOS> <swiftc args...>: one slice per architecture, joined with lipo.
swift_build() {
    local output="$1" minos="$2" slices=() arch
    shift 2
    for arch in "${ARCH_LIST[@]}"; do
        swiftc -O -target "${arch}-apple-macos${minos}" "$@" -o "${WORK_DIR}/$(basename "${output}").${arch}"
        slices+=("${WORK_DIR}/$(basename "${output}").${arch}")
    done
    if [[ "${#slices[@]}" -eq 1 ]]; then
        cp "${slices[0]}" "${output}"
    else
        lipo -create -output "${output}" "${slices[@]}"
    fi
    chmod 0755 "${output}"
}

swift_build "${EXT_MACOS}/${EXT_ID}" 12.3 \
    -parse-as-library \
    -framework CoreMedia \
    -framework CoreMediaIO \
    -framework CoreVideo \
    -framework Foundation \
    -framework IOKit \
    "${SCRIPT_DIR}/NectarLinkCamera/SharedFrameReader.swift" \
    "${SCRIPT_DIR}/NectarLinkCamera/CameraProvider.swift" \
    "${SCRIPT_DIR}/NectarLinkCamera/main.swift"

sed -e "s/@VERSION@/${VERSION}/g" \
    -e "s/@BUNDLE_VERSION@/${BUNDLE_VERSION}/g" \
    -e "s/@APP_GROUP_ID@/${GROUP_ID}/g" \
    "${SCRIPT_DIR}/NectarLinkCamera/Info.plist.in" > "${EXT_CONTENTS}/Info.plist"
plutil -lint "${EXT_CONTENTS}/Info.plist" >/dev/null

if [[ -n "${EXT_PROFILE:-}" ]]; then
    cp "${EXT_PROFILE}" "${EXT_CONTENTS}/embedded.provisionprofile"
fi

# render_entitlements <template> <output> <bundle ID> <identity keys: 1|0>:
# $(TeamIdentifierPrefix) becomes the team ID and a dot (nothing without a team),
# as Xcode would do.
render_entitlements() {
    local template="$1" output="$2" bundle_id="$3" identity_keys="$4"
    sed -e "s/\$(TeamIdentifierPrefix)/${TEAM_ID:+${TEAM_ID}.}/g" "${template}" > "${output}"
    if [[ "${identity_keys}" == "1" ]]; then
        [[ -n "${TEAM_ID}" ]] || { echo "error: PROFILE_KEYS=1 needs TEAM_ID" >&2; exit 1; }
        "${PLISTBUDDY}" -c "Add :com.apple.application-identifier string ${TEAM_ID}.${bundle_id}" "${output}"
        "${PLISTBUDDY}" -c "Add :com.apple.developer.team-identifier string ${TEAM_ID}" "${output}"
    fi
    plutil -lint "${output}" >/dev/null
}

render_entitlements "${SCRIPT_DIR}/NectarLinkCamera/NectarLinkCamera.entitlements" \
    "${OUT_DIR}/NectarLinkCamera.entitlements" "${EXT_ID}" "$([[ -n "${EXT_PROFILE:-}" ]] && echo 1 || echo 0)"
render_entitlements "${SCRIPT_DIR}/HostApp.entitlements" "${OUT_DIR}/HostApp.entitlements" "${HOST_BUNDLE_ID}" \
    "${PROFILE_KEYS:-0}"

if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
    keychain=()
    [[ -z "${KEYCHAIN_PATH:-}" ]] || keychain=(--keychain "${KEYCHAIN_PATH}")
    codesign --force --options runtime --timestamp ${keychain[@]+"${keychain[@]}"} \
        --entitlements "${OUT_DIR}/NectarLinkCamera.entitlements" \
        --sign "${CODESIGN_IDENTITY}" "${EXT_BUNDLE}"
fi

echo "Built ${EXT_BUNDLE} (${ARCH_LIST[*]}, group ${GROUP_ID})"
