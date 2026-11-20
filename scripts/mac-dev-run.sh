#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the desktop app and runs it from a minimal .app bundle, for trying
# what only works in a bundle: native notifications (UserNotifications refuses
# to work for a bare `cargo run` binary), the Keychain's access prompts and
# the app's name in the menu bar and System Settings.
#
#   scripts/mac-dev-run.sh [app arguments]
#
# It has its own bundle identifier (app.nectarlink.desktop.dev), so the
# permissions macOS asks for are separate from an installed NectarLink's.
# Qt is found where the build found it (Homebrew or QT_ROOT_DIR), so there's
# no macdeployqt step; use desktop/packaging/macos/build-dmg.sh for a
# distributable app.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

PROFILE_DIR="debug"
CARGO_ARGS=()
if [[ "${RELEASE:-0}" == "1" ]]; then
    PROFILE_DIR="release"
    CARGO_ARGS=(--release)
fi

cargo build --locked "${CARGO_ARGS[@]}" -p nectarlink-desktop

VERSION="$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' Cargo.toml)"
APP="${REPO_ROOT}/target/${PROFILE_DIR}/NectarLink-dev.app"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"

sed -e "s/@VERSION@/${VERSION}/g" -e "s/@BUNDLE_VERSION@/${VERSION%%-*}/g" \
    -e 's/app\.nectarlink\.desktop/app.nectarlink.desktop.dev/' \
    -e 's|<string>NectarLink</string>|<string>NectarLink Dev</string>|' \
    desktop/packaging/macos/Info.plist.in > "${APP}/Contents/Info.plist"

# A clone on APFS, so a big debug binary costs no time or space.
cp -c "target/${PROFILE_DIR}/nectarlink-desktop" "${APP}/Contents/MacOS/nectarlink-desktop"
for qm in target/"${PROFILE_DIR}"/nectarlink_*.qm; do
    [[ ! -f "${qm}" ]] || cp "${qm}" "${APP}/Contents/Resources/"
done

# Ad-hoc by default, which macOS may refuse to send notifications for; set
# SIGN_IDENTITY (a "Developer ID Application: ..." name or hash) to sign properly.
codesign --force --sign "${SIGN_IDENTITY:--}" --timestamp=none "${APP}"

# Run the binary itself, so its log stays in this terminal; the system still
# sees the bundle around it.
exec "${APP}/Contents/MacOS/nectarlink-desktop" "$@"
