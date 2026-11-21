#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tells a Discord channel about a nightly or a release, through its webhook:
a heading, the top changes as short points, and the link, with one role
pinged. Notes come from commit and pull request titles, which anyone can
write, so the message may ping that role and nobody else.

Usage: discord-post.py <heading> <link> < notes.md
Reads DISCORD_WEBHOOK and DISCORD_ROLE_ID from the environment. Without a
webhook it does nothing, and a failed post is a warning, never a failure:
the release itself is out either way."""

import json
import os
import re
import sys
import urllib.request

MAX_POINTS = 8
MAX_POINT_CHARS = 120
# Discord's limit is 2,000 characters; the rest is room for the heading.
MAX_CHARS = 1900

BULLET = re.compile(r"^\s*[-*]\s+(.+?)\s*$")
BOLD_LEAD = re.compile(r"^\*\*(.+?)\*\*")
REFERENCES = re.compile(r"\s*\((?:#\d+(?:,\s*)?)+\)|\s*\[#\d+\]\([^)]*\)")


def point(line: str) -> str:
    """One short line for a change: a changelog entry's bold lead when it has
    one, else the text, without issue numbers."""
    lead = BOLD_LEAD.match(line)
    text = lead.group(1) if lead else line
    text = REFERENCES.sub("", text).strip()
    # Nothing in it can ping or link anyone by accident: allowed_mentions
    # below stops the pings, and this stops "@everyone" showing as one.
    text = text.replace("@", "@​")
    if len(text) > MAX_POINT_CHARS:
        text = text[: MAX_POINT_CHARS - 1].rstrip() + "…"
    return text


def points(notes: str) -> tuple[list[str], int]:
    """The first MAX_POINTS changes, and how many more there were."""
    found = [point(m.group(1)) for line in notes.splitlines() if (m := BULLET.match(line))]
    found = [p for p in found if p]
    return found[:MAX_POINTS], max(0, len(found) - MAX_POINTS)


def message(heading: str, link: str, notes: str, role: str) -> dict:
    shown, more = points(notes)
    lines = []
    if role:
        lines.append(f"<@&{role}>")
    lines.append(f"## {heading}")
    lines.extend(f"- {p}" for p in shown)
    if more:
        lines.append(f"-# and {more} more")
    body = "\n".join(lines)
    if len(body) > MAX_CHARS:
        body = body[: MAX_CHARS - 1].rstrip() + "…"
    content = f"{body}\n\n{link}"
    return {
        "content": content,
        "allowed_mentions": {"parse": [], "roles": [role] if role else []},
    }


def main() -> None:
    heading, link = sys.argv[1:3]
    webhook = os.environ.get("DISCORD_WEBHOOK", "").strip()
    if not webhook:
        print("no Discord webhook set; not posting")
        return
    role = os.environ.get("DISCORD_ROLE_ID", "").strip()
    if role and not role.isdigit():
        print("::warning::DISCORD_ROLE_ID isn't a role ID; posting without a ping")
        role = ""
    payload = json.dumps(message(heading, link, sys.stdin.read(), role)).encode()
    request = urllib.request.Request(
        webhook,
        data=payload,
        headers={"Content-Type": "application/json", "User-Agent": "nectarlink-release"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            print(f"posted to Discord ({response.status})")
    except Exception as error:  # noqa: BLE001 - any failure is only a warning
        # The URL holds the webhook's token, so it never goes in the log.
        print(f"::warning::couldn't post to Discord: {type(error).__name__} {getattr(error, 'code', '')}")


if __name__ == "__main__":
    main()
