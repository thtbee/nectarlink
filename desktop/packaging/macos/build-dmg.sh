#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds a macOS .app bundle and compressed .dmg installer for NectarLink Desktop:
#   dist/Nectarlink-<version>-macos-<arch>.dmg
#
# ARCH: universal (default), arm64 or x86_64. Universal needs a universal Qt
# (the Qt installer's; Homebrew's is for your Mac's chip only).
#
# Signing is optional; without it the bundle is ad-hoc signed ("Open Anyway").
#   SIGN_IDENTITY   "Developer ID Application: ..." name or hash
#                   (KEYCHAIN_PATH: the keychain holding it)
#   NOTARY_PROFILE  a `notarytool store-credentials` profile, or
#   APPLE_API_KEY_PATH, APPLE_API_KEY_ID, APPLE_API_ISSUER  an App Store Connect key
# With both, the app and the DMG are notarized and stapled.
# REQUIRE_SIGNING=1 makes missing credentials an error.
#
# The phone-as-webcam virtual camera is a CoreMediaIO camera extension built
# from desktop/vcam/macos and embedded in the app, which installs it itself.
# That needs a provisioning profile from the Apple Developer portal (see
# desktop/vcam/macos/README.md), given as a file:
#   HOST_PROVISIONING_PROFILE  .provisionprofile for app.nectarlink.desktop
#                              (Developer ID, System Extension capability). Needs
#                              SIGN_IDENTITY to be the certificate it was made for.
#   VCAM_PROVISIONING_PROFILE  optional, .provisionprofile for
#                              app.nectarlink.desktop.camera-extension; the
#                              extension works without one
#   APPLE_TEAM_ID              the team ID (read from the profile or identity if unset)
#   REQUIRE_VCAM=1             error out unless the host profile is given and used
#   SKIP_VCAM=1                leave the camera extension out of the app
# Without the profiles the extension is still embedded and signed, but the app
# can't install it: Settings > Virtual camera > Set up reports that.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DESKTOP_DIR="${REPO_ROOT}/desktop/app"
ENTITLEMENTS="${SCRIPT_DIR}/NectarLink.entitlements"

VERSION="${VERSION:-$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "${REPO_ROOT}/Cargo.toml")}"
if [[ -z "${VERSION}" ]]; then
    echo "error: could not determine version from ${REPO_ROOT}/Cargo.toml" >&2
    exit 1
fi
# CFBundleVersion requires numeric dot-separated components (strip -nightly.N suffix if present).
BUNDLE_VERSION="${VERSION%%-*}"

ARCH="${ARCH:-universal}"
case "${ARCH}" in
    universal)
        RUST_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
        DMG_ARCH="universal"
        EXPECTED_ARCHS="arm64 x86_64"
        ;;
    arm64|aarch64)
        RUST_TARGETS=(aarch64-apple-darwin)
        DMG_ARCH="arm64"
        EXPECTED_ARCHS="arm64"
        ;;
    x86_64|amd64)
        RUST_TARGETS=(x86_64-apple-darwin)
        DMG_ARCH="x86_64"
        EXPECTED_ARCHS="x86_64"
        ;;
    *)
        echo "error: unsupported ARCH '${ARCH}' (universal, arm64 or x86_64)" >&2
        exit 1
        ;;
esac

if [[ -n "${BIN_PATH:-}" && "${#RUST_TARGETS[@]}" -gt 1 ]]; then
    echo "error: BIN_PATH names one binary, so it needs ARCH=arm64 or ARCH=x86_64" >&2
    exit 1
fi

# Matches LSMinimumSystemVersion in Info.plist, and Qt 6.12's own floor.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.4}"

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

# --- Signing and notarization settings ---------------------------------------

SIGN_IDENTITY="${SIGN_IDENTITY:-}"
KEYCHAIN_ARGS=()
[[ -z "${KEYCHAIN_PATH:-}" ]] || KEYCHAIN_ARGS=(--keychain "${KEYCHAIN_PATH}")

NOTARY_ARGS=()
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    NOTARY_ARGS=(--keychain-profile "${NOTARY_PROFILE}")
elif [[ -n "${APPLE_API_KEY_PATH:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER:-}" ]]; then
    NOTARY_ARGS=(--key "${APPLE_API_KEY_PATH}" --key-id "${APPLE_API_KEY_ID}" --issuer "${APPLE_API_ISSUER}")
fi

if [[ "${REQUIRE_SIGNING:-0}" == "1" ]]; then
    if [[ -z "${SIGN_IDENTITY}" ]]; then
        echo "error: REQUIRE_SIGNING=1 but SIGN_IDENTITY is not set" >&2
        exit 1
    fi
    if [[ "${#NOTARY_ARGS[@]}" -eq 0 ]]; then
        echo "error: REQUIRE_SIGNING=1 but no notarization credentials are set" >&2
        exit 1
    fi
fi
if [[ -n "${SIGN_IDENTITY}" && "${#NOTARY_ARGS[@]}" -eq 0 ]]; then
    echo "warning: signing without notarization credentials; Gatekeeper will still warn" >&2
fi
if [[ -z "${SIGN_IDENTITY}" ]]; then
    echo "warning: no SIGN_IDENTITY; the bundle is only ad-hoc signed" >&2
fi

# sign_path <path> [entitlements]: Developer ID with the hardened runtime, or
# ad-hoc without it (it would refuse the bundled Qt frameworks).
sign_path() {
    local target="$1" entitlements="${2:-}"
    local args=(--force --sign "${SIGN_IDENTITY:--}")
    if [[ -n "${SIGN_IDENTITY}" ]]; then
        args+=(--options runtime --timestamp "${KEYCHAIN_ARGS[@]}")
        [[ -z "${entitlements}" ]] || args+=(--entitlements "${entitlements}")
    else
        args+=(--timestamp=none)
    fi
    codesign "${args[@]}" "${target}"
}

# notarize <path>: waits; a rejection prints Apple's log and fails.
notarize() {
    local path="$1" out
    echo "Notarizing $(basename "${path}")..."
    out="$(xcrun notarytool submit "${path}" "${NOTARY_ARGS[@]}" --wait --output-format json)"
    if [[ "$(printf '%s' "${out}" | plutil -extract status raw -o - - 2>/dev/null)" != "Accepted" ]]; then
        local id
        id="$(printf '%s' "${out}" | plutil -extract id raw -o - - 2>/dev/null || true)"
        echo "error: notarization of $(basename "${path}") was not accepted" >&2
        [[ -z "${id}" ]] || xcrun notarytool log "${id}" "${NOTARY_ARGS[@]}" >&2 || true
        exit 1
    fi
}

# --- Binaries ----------------------------------------------------------------

for target in "${RUST_TARGETS[@]}"; do
    if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -qx "${target}"; then
        rustup target add "${target}"
    fi
done

# release_bin <target> <name>
release_bin() {
    echo "${REPO_ROOT}/target/$1/release/$2"
}

for target in "${RUST_TARGETS[@]}"; do
    desktop_bin="${BIN_PATH:-$(release_bin "${target}" nectarlink-desktop)}"
    if [[ ! -f "${desktop_bin}" ]]; then
        echo "Building nectarlink-desktop (release, ${target})..."
        (cd "${REPO_ROOT}" && cargo build --locked --release --target "${target}" -p nectarlink-desktop -p nectarlink-mcp)
    fi
done

STAGE_DIR="$(mktemp -d -t nectarlink-macos-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT

APP_DIR="${STAGE_DIR}/NectarLink.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

# install_binary <name> <destination> <required|optional>: copies the binary,
# or joins the per-target ones with lipo.
install_binary() {
    local name="$1" dest="$2" required="$3" sources=() target source
    for target in "${RUST_TARGETS[@]}"; do
        if [[ "${name}" == "nectarlink-desktop" && -n "${BIN_PATH:-}" ]]; then
            source="${BIN_PATH}"
        elif [[ "${name}" == "nectarlink-agent" && -n "${AGENT_PATH:-}" ]]; then
            source="${AGENT_PATH}"
        else
            source="$(release_bin "${target}" "${name}")"
        fi
        if [[ -f "${source}" ]]; then
            sources+=("${source}")
        elif [[ "${required}" == "required" ]]; then
            echo "error: ${source} not found" >&2
            exit 1
        else
            return 0
        fi
    done
    if [[ "${#sources[@]}" -eq 1 ]]; then
        install -m 0755 "${sources[0]}" "${dest}"
    else
        lipo -create -output "${dest}" "${sources[@]}"
        chmod 0755 "${dest}"
    fi
}

install_binary nectarlink-desktop "${MACOS_DIR}/nectarlink-desktop" required
install_binary nectarlink-agent "${MACOS_DIR}/nectarlink-agent" optional

# --- Virtual camera extension ------------------------------------------------

HOST_BUNDLE_ID="app.nectarlink.desktop"
VCAM_DIR="${REPO_ROOT}/desktop/vcam/macos"
VCAM_BUILD="${STAGE_DIR}/vcam"
EXT_NAME="${HOST_BUNDLE_ID}.camera-extension.systemextension"
EXT_DEST="${CONTENTS_DIR}/Library/SystemExtensions/${EXT_NAME}"
HOST_ENTITLEMENTS="${ENTITLEMENTS}"
VCAM_PROFILES=0

profile_team_id() {
    security cms -D -i "$1" 2>/dev/null | plutil -extract 'TeamIdentifier.0' raw -o - - 2>/dev/null || true
}

if [[ "${SKIP_VCAM:-0}" != "1" ]]; then
    HOST_PROFILE="${HOST_PROVISIONING_PROFILE:-}"
    VCAM_PROFILE="${VCAM_PROVISIONING_PROFILE:-}"
    for profile in "${HOST_PROFILE}" "${VCAM_PROFILE}"; do
        if [[ -n "${profile}" && ! -f "${profile}" ]]; then
            echo "error: provisioning profile ${profile} not found" >&2
            exit 1
        fi
    done
    if [[ -n "${HOST_PROFILE}" ]]; then
        if [[ -n "${SIGN_IDENTITY}" ]]; then
            VCAM_PROFILES=1
        else
            echo "warning: the provisioning profile needs SIGN_IDENTITY; building the camera extension without it" >&2
        fi
    elif [[ -n "${VCAM_PROFILE}" ]]; then
        echo "error: VCAM_PROVISIONING_PROFILE needs HOST_PROVISIONING_PROFILE too" >&2
        exit 1
    fi
    if [[ "${REQUIRE_VCAM:-0}" == "1" && "${VCAM_PROFILES}" != "1" ]]; then
        echo "error: REQUIRE_VCAM=1 but HOST_PROVISIONING_PROFILE and SIGN_IDENTITY are not both set" >&2
        exit 1
    fi

    # The team ID names the App Group: explicit, else the profile's, else the
    # signing identity's "(TEAMID)" (looked up when it is a hash).
    TEAM_ID="${APPLE_TEAM_ID:-}"
    [[ -n "${TEAM_ID}" || -z "${HOST_PROFILE}" ]] || TEAM_ID="$(profile_team_id "${HOST_PROFILE}")"
    if [[ -z "${TEAM_ID}" && -n "${SIGN_IDENTITY}" ]]; then
        TEAM_ID="$(security find-identity -v -p codesigning ${KEYCHAIN_PATH:+"${KEYCHAIN_PATH}"} 2>/dev/null \
            | grep -F "${SIGN_IDENTITY}" | sed -n 's/.*(\([A-Z0-9]\{10\}\))".*/\1/p' | head -n 1 || true)"
        if [[ -z "${TEAM_ID}" ]]; then
            TEAM_ID="$(printf '%s' "${SIGN_IDENTITY}" | sed -n 's/.*(\([A-Z0-9]\{10\}\)).*/\1/p')"
        fi
    fi
    if [[ -z "${TEAM_ID}" && "${VCAM_PROFILES}" == "1" ]]; then
        echo "error: can't tell the Apple team ID; set APPLE_TEAM_ID" >&2
        exit 1
    fi

    echo "Building the virtual camera extension (${DMG_ARCH}${TEAM_ID:+, team ${TEAM_ID}})..."
    ARCHS="${DMG_ARCH}" VERSION="${VERSION}" BUNDLE_VERSION="${BUNDLE_VERSION}" TEAM_ID="${TEAM_ID}" \
        HOST_BUNDLE_ID="${HOST_BUNDLE_ID}" EXT_PROFILE="${VCAM_PROFILE}" \
        PROFILE_KEYS="$([[ "${VCAM_PROFILES}" == "1" ]] && echo 1 || echo 0)" \
        bash "${VCAM_DIR}/build-vcam-extension.sh" "${VCAM_BUILD}"

    mkdir -p "${CONTENTS_DIR}/Library/SystemExtensions"
    cp -R "${VCAM_BUILD}/${EXT_NAME}" "${EXT_DEST}"

    if [[ "${VCAM_PROFILES}" == "1" ]]; then
        cp "${HOST_PROFILE}" "${CONTENTS_DIR}/embedded.provisionprofile"
        # The app's own entitlements plus the ones the profile backs.
        HOST_ENTITLEMENTS="${STAGE_DIR}/NectarLink.host.entitlements"
        cp "${ENTITLEMENTS}" "${HOST_ENTITLEMENTS}"
        /usr/libexec/PlistBuddy -c "Merge ${VCAM_BUILD}/HostApp.entitlements" "${HOST_ENTITLEMENTS}"
    else
        echo "warning: no provisioning profile: the camera extension is bundled but macOS will refuse to install it" >&2
    fi
fi

# Translations go in Resources: Contents/MacOS is for code only, which
# is what codesign expects there. Compiled here from the catalogues with
# Qt's lrelease; the ones the build left next to the binary otherwise.
LRELEASE="${LRELEASE:-}"
if [[ -z "${LRELEASE}" ]]; then
    for candidate in "${QT_ROOT_DIR:-}/bin/lrelease" "$(dirname "${MACDEPLOYQT}")/lrelease" "$(command -v lrelease || true)"; do
        if [[ -n "${candidate}" && -x "${candidate}" ]]; then
            LRELEASE="${candidate}"
            break
        fi
    done
fi
qm_count=0
if [[ -n "${LRELEASE}" ]]; then
    for ts in "${DESKTOP_DIR}"/i18n/nectarlink_*.ts; do
        "${LRELEASE}" -silent "${ts}" -qm "${RESOURCES_DIR}/$(basename "${ts%.ts}").qm"
        qm_count=$((qm_count + 1))
    done
else
    for qm in "$(dirname "$(release_bin "${RUST_TARGETS[0]}" nectarlink-desktop)")"/nectarlink_*.qm; do
        if [[ -f "${qm}" ]]; then
            install -m 0644 "${qm}" "${RESOURCES_DIR}/$(basename "${qm}")"
            qm_count=$((qm_count + 1))
        fi
    done
fi
if [[ "${qm_count}" -eq 0 ]]; then
    echo "error: no translations to bundle (set LRELEASE or QT_ROOT_DIR)" >&2
    exit 1
fi
echo "Bundled ${qm_count} translations"

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

# --- Signing -----------------------------------------------------------------

# Inside out: libraries and frameworks before the app (--deep is deprecated).
echo "Signing ${SIGN_IDENTITY:+with ${SIGN_IDENTITY}}${SIGN_IDENTITY:-ad-hoc}..."
while IFS= read -r -d '' lib; do
    sign_path "${lib}"
done < <(find "${CONTENTS_DIR}" -type f \( -name '*.dylib' -o -name '*.so' \) -print0)
if [[ -d "${CONTENTS_DIR}/Frameworks" ]]; then
    while IFS= read -r -d '' framework; do
        sign_path "${framework}"
    done < <(find "${CONTENTS_DIR}/Frameworks" -maxdepth 1 -name '*.framework' -print0)
fi
[[ ! -f "${MACOS_DIR}/nectarlink-agent" ]] || sign_path "${MACOS_DIR}/nectarlink-agent"
if [[ -d "${EXT_DEST:-}" ]]; then
    sign_path "${EXT_DEST}" "${VCAM_BUILD}/NectarLinkCamera.entitlements"
fi
sign_path "${MACOS_DIR}/nectarlink-desktop" "${HOST_ENTITLEMENTS}"
sign_path "${APP_DIR}" "${HOST_ENTITLEMENTS}"

codesign --verify --deep --strict --verbose=2 "${APP_DIR}"
ACTUAL_ARCHS="$(lipo -archs "${MACOS_DIR}/nectarlink-desktop" | tr ' ' '\n' | sort | tr '\n' ' ' | sed 's/ $//')"
if [[ "${ACTUAL_ARCHS}" != "${EXPECTED_ARCHS}" ]]; then
    echo "error: nectarlink-desktop has architectures '${ACTUAL_ARCHS}', expected '${EXPECTED_ARCHS}'" >&2
    exit 1
fi

if [[ -d "${EXT_DEST:-}" ]]; then
    ACTUAL_ARCHS="$(lipo -archs "${EXT_DEST}/Contents/MacOS/${EXT_NAME%.systemextension}" | tr ' ' '\n' | sort | tr '\n' ' ' | sed 's/ $//')"
    if [[ "${ACTUAL_ARCHS}" != "${EXPECTED_ARCHS}" ]]; then
        echo "error: the camera extension has architectures '${ACTUAL_ARCHS}', expected '${EXPECTED_ARCHS}'" >&2
        exit 1
    fi
    codesign --verify --strict --verbose=2 "${EXT_DEST}"
    echo "Bundled the camera extension: ${EXT_DEST#"${APP_DIR}"/}"
fi

if [[ -n "${SIGN_IDENTITY}" && "${#NOTARY_ARGS[@]}" -gt 0 ]]; then
    # Notarize and staple the app too, so it opens offline once dragged out.
    ditto -c -k --keepParent "${APP_DIR}" "${STAGE_DIR}/NectarLink.zip"
    notarize "${STAGE_DIR}/NectarLink.zip"
    rm -f "${STAGE_DIR}/NectarLink.zip"
    xcrun stapler staple "${APP_DIR}"
    spctl --assess --type execute --verbose=2 "${APP_DIR}"
fi

# --- DMG ---------------------------------------------------------------------

DMG_SRC="$(mktemp -d -t nectarlink-dmg-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}" "${DMG_SRC}"' EXIT
mv "${APP_DIR}" "${DMG_SRC}/NectarLink.app"
ln -s /Applications "${DMG_SRC}/Applications"

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
mkdir -p "${OUT_DIR}"
DMG_PATH="${OUT_DIR}/Nectarlink-${VERSION}-macos-${DMG_ARCH}.dmg"
rm -f "${DMG_PATH}"

echo "Creating ${DMG_PATH}..."
hdiutil create -volname "NectarLink ${VERSION}" \
    -srcfolder "${DMG_SRC}" \
    -ov -format UDZO \
    "${DMG_PATH}" >/dev/null

if [[ -n "${SIGN_IDENTITY}" ]]; then
    sign_path "${DMG_PATH}"
    if [[ "${#NOTARY_ARGS[@]}" -gt 0 ]]; then
        notarize "${DMG_PATH}"
        xcrun stapler staple "${DMG_PATH}"
        xcrun stapler validate "${DMG_PATH}"
        spctl --assess --type open --context context:primary-signature --verbose=2 "${DMG_PATH}"
    fi
fi

echo "Built ${DMG_PATH}"
