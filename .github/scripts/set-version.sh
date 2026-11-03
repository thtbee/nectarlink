#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Sets the workspace's version (a nightly's, say 0.1.5-nightly.7) in
# Cargo.toml: the [workspace.package] version and our own crates' entries in
# [workspace.dependencies], which must match it. Nothing else changes.
set -euo pipefail

new="${1:?usage: set-version.sh <version>}"
if ! [[ "${new}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-nightly\.[0-9]+)?$ ]]; then
    echo "error: ${new} is neither a release nor a nightly version" >&2
    exit 1
fi
manifest="$(dirname "$0")/../../Cargo.toml"
old="$(awk -F'"' '/^version = "/ { print $2; exit }' "${manifest}")"
if [[ -z "${old}" ]]; then
    echo "error: no version in ${manifest}" >&2
    exit 1
fi
[[ "${old}" == "${new}" ]] && exit 0
# Dots in the old version are matched literally.
pattern="${old//./\\.}"
sed -i -e "s/^version = \"${pattern}\"/version = \"${new}\"/" \
    -e "/path = /s/version = \"${pattern}\"/version = \"${new}\"/" "${manifest}"
grep -c "\"${new}\"" "${manifest}" >/dev/null
# Our own crates' entries in Cargo.lock follow, so `--locked` builds still
# pass; no dependency changes version.
(cd "$(dirname "${manifest}")" && { cargo update --workspace --offline 2>/dev/null || cargo update --workspace; })
echo "version ${old} -> ${new}"
