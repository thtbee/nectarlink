#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Writes a channel's feed (desktop/app/src/feed.rs) for a published release:
its version, name, notes page, and each file with its download address and
the SHA-256 from the release's SHA256SUMS.txt. Only files the sums list are
in it, so the apps never get a file without a hash to check.

    write-feed.py <stable|nightly> <tag> <name> <out.json>

Reads the release with `gh` (GH_TOKEN, GITHUB_REPOSITORY)."""

import json
import os
import re
import subprocess
import sys

SCHEMA = 1


def gh(*args: str) -> str:
    return subprocess.check_output(["gh", *args, "--repo", os.environ["GITHUB_REPOSITORY"]], text=True)


def main() -> None:
    channel, tag, name, out = sys.argv[1:5]
    if channel not in ("stable", "nightly"):
        sys.exit(f"unknown channel {channel}")
    version = tag.removeprefix("v")
    nightly = re.fullmatch(r"\d+\.\d+\.\d+-nightly\.\d+", version)
    if (channel == "nightly") != bool(nightly) or not re.fullmatch(r"\d+\.\d+\.\d+(-nightly\.\d+)?", version):
        sys.exit(f"{tag} isn't a {channel} build")
    release = json.loads(gh("release", "view", tag, "--json", "url,publishedAt,assets,isDraft"))
    if release["isDraft"]:
        sys.exit(f"{tag} isn't published")
    sums_text = gh("release", "download", tag, "--pattern", "SHA256SUMS.txt", "--output", "-")
    sums = {}
    for line in sums_text.splitlines():
        parts = line.split()
        if len(parts) == 2 and re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            sums[parts[1].lstrip("*")] = parts[0]
    prefix = f"https://github.com/{os.environ['GITHUB_REPOSITORY']}/releases/download/"
    files = [
        {"name": a["name"], "url": a["url"], "sha256": sums[a["name"]], "size": a["size"]}
        for a in release["assets"]
        if a["name"] in sums and a["url"].startswith(prefix)
    ]
    if not files:
        sys.exit(f"{tag} has no files listed in its sums")
    feed = {
        "schema": SCHEMA,
        "channel": channel,
        "version": version,
        "name": name or None,
        "published": release["publishedAt"],
        "notes": release["url"],
        "files": sorted(files, key=lambda f: f["name"]),
    }
    with open(out, "w", encoding="utf-8") as f:
        json.dump(feed, f, indent=2)
        f.write("\n")
    print(json.dumps(feed, indent=2))


if __name__ == "__main__":
    main()
