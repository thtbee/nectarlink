#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Puts a channel's feed (write-feed.py's output) on the `channels` branch,
# where the apps read it (raw.githubusercontent.com/<repo>/channels/<channel>.json).
# The branch holds only the feeds; it's created on first use. A push that
# races another workflow's is retried on top of it.
#
#   publish-feed.sh <stable|nightly> <feed.json>
set -euo pipefail

channel="${1:?usage: publish-feed.sh <channel> <feed.json>}"
feed="$(realpath "${2:?usage: publish-feed.sh <channel> <feed.json>}")"
case "${channel}" in stable|nightly) ;; *) echo "error: unknown channel ${channel}" >&2; exit 1 ;; esac
version="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "${feed}")"

dir="$(mktemp -d)"
trap 'rm -rf -- "${dir}"' EXIT
git -C "${dir}" init -q
git -C "${dir}" config user.name "github-actions[bot]"
git -C "${dir}" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git -C "${dir}" remote add origin "https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPOSITORY}.git"

for attempt in 1 2 3 4 5; do
    if git -C "${dir}" fetch -q --depth 1 origin channels 2>/dev/null; then
        git -C "${dir}" checkout -q -B channels FETCH_HEAD
    else
        git -C "${dir}" checkout -q --orphan channels
        git -C "${dir}" rm -rfq --cached . 2>/dev/null || true
        printf '%s\n' \
            "# Update channels" "" \
            "The newest build on each channel, for NectarLink's updaters to read" \
            "(see \`desktop/app/src/feed.rs\` on main). Written by the release and" \
            "nightly workflows; don't edit by hand." > "${dir}/README.md"
        git -C "${dir}" add README.md
    fi
    cp "${feed}" "${dir}/${channel}.json"
    git -C "${dir}" add "${channel}.json"
    if git -C "${dir}" diff --cached --quiet; then
        echo "${channel}.json already says ${version}"
        exit 0
    fi
    git -C "${dir}" commit -q -m "${channel}: ${version}"
    if git -C "${dir}" push -q origin channels; then
        echo "published ${channel}.json: ${version}"
        exit 0
    fi
    echo "push raced another; trying again (${attempt})" >&2
    sleep $((attempt * 3))
done
echo "error: couldn't publish ${channel}.json" >&2
exit 1
