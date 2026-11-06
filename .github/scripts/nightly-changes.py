#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""What a nightly changes for people, from the commits since the last one:
feat, fix and perf subjects as a Markdown list, without docs, CI and chores.
Usage: nightly-changes.py <previous tag> <commit>. Prints nothing if there's
nothing to list."""

import re
import subprocess
import sys

# "fix(android)!: the thing" -> "the thing".
COMMIT = re.compile(r"^(?:feat|fix|perf)(?:\([^)]*\))?!?:\s*(.+)$")


def changes(subjects: list[str]) -> list[str]:
    """The lines to list, newest first, each once."""
    lines: list[str] = []
    for subject in subjects:
        match = COMMIT.match(subject.strip())
        if not match:
            continue
        text = match.group(1)
        line = f"- {text[0].upper()}{text[1:]}"
        if line not in lines:
            lines.append(line)
    return lines


def main() -> None:
    previous, commit = sys.argv[1:3]
    log = subprocess.run(
        ["git", "log", "--format=%s", f"{previous}..{commit}"],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    for line in changes(log.stdout.splitlines()):
        print(line)


if __name__ == "__main__":
    main()
