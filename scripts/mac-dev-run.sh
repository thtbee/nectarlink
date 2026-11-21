#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the desktop app and runs it from a minimal .app bundle, for what
# needs one (native notifications). Not distributable: use build-dmg.sh.
#
#   scripts/mac-dev-run.sh [app arguments]
#
# The bundle id is app.nectarlink.desktop.dev, so permissions are separate
# from an installed NectarLink's.

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

# An APFS clone: free, even for a big debug binary.
cp -c "target/${PROFILE_DIR}/nectarlink-desktop" "${APP}/Contents/MacOS/nectarlink-desktop"
for qm in target/"${PROFILE_DIR}"/nectarlink_*.qm; do
    [[ ! -f "${qm}" ]] || cp "${qm}" "${APP}/Contents/Resources/"
done

# Ad-hoc by default; SIGN_IDENTITY signs with a real identity, which keeps
# the Keychain's "Always Allow" across rebuilds.
codesign --force --sign "${SIGN_IDENTITY:--}" --timestamp=none "${APP}"

# Run the binary itself, so the log stays in this terminal.
exec "${APP}/Contents/MacOS/nectarlink-desktop" "$@"
