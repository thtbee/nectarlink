#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Keeps log lines in the Rust code private by construction.

The activity log (nectarlink_core::diag) hides any field it doesn't know is
safe, and strips folders and addresses from text. Two things it can't check
are checked here:

1. A log message is fixed text. `warn!("can't open {path}")` puts a value in
   the message, where no field rule applies; values go in fields instead:
   `warn!(error = %e, "can't open the file")`.
2. A device is logged by its short ID (`peer = %id.short()`), never the
   whole one (`%peer`, `device = %d.id`).

A line that has to do otherwise says why, with `// log-ok: <reason>` on it
or the line before. Unit tests (`#[cfg(test)] mod tests`) aren't checked.

Usage: check_log_privacy.py [root]. Exits 1 and lists what's wrong.
"""

import re
import sys
from pathlib import Path

MACRO = re.compile(r"\b(?:tracing::)?(trace|debug|info|warn|error)!\(")
STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')
DEVICE_FIELD = re.compile(r"\b(peer|device|from)\s*=\s*%\s*([^,]+?)\s*(?:,|$)")
# `%peer` is short for `peer = %peer`: the whole ID.
DEVICE_SHORTHAND = re.compile(r"(?<![\w.])[%?]\s*(peer|device|from)\b(?!\s*[.=(])")
VALUE_IN_TEXT = re.compile(r"(?<!\{)\{(?!\{)")
TESTS = re.compile(r"#\[cfg\(test\)\]\s*mod tests")
DIRS = ("core", "desktop")


def macro_bodies(source):
    """(line number, body) for each tracing macro call in `source`."""
    for m in MACRO.finditer(source):
        i, depth = m.end(), 1
        while i < len(source) and depth:
            c = source[i]
            if c == '"':
                s = STRING.match(source, i)
                i = s.end() if s else i + 1
                continue
            if c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
            i += 1
        yield source.count("\n", 0, m.start()) + 1, source[m.end() : i - 1]


def top_level_strings(body):
    """(start, text) of each string literal in `body` outside any brackets,
    so `format!("{:.1}", x)` as a field's value isn't taken for the message."""
    i, depth = 0, 0
    while i < len(body):
        c = body[i]
        if c == '"':
            m = STRING.match(body, i)
            if not m:
                return
            if depth == 0:
                yield i, m.group(1)
            i = m.end()
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        i += 1


def message(body):
    """The message: the first top-level string that isn't a field's value."""
    for start, text in top_level_strings(body):
        before = body[:start].rstrip()
        if before.endswith("=") or before.endswith("target:"):
            continue
        return text
    return None


def without_tests(source):
    """`source` up to its unit tests."""
    m = TESTS.search(source)
    return source[: m.start()] if m else source


def problems(path, source):
    lines = source.splitlines()
    for line_no, body in macro_bodies(source):
        span = "\n".join(lines[max(0, line_no - 2) : line_no + body.count("\n")])
        if "log-ok:" in span:
            continue
        text = message(body)
        if text is not None and VALUE_IN_TEXT.search(text):
            yield f"{path}:{line_no}: the log message has a value in it; put it in a field: {text!r}"
        flat = " ".join(body.split())
        for name, value in DEVICE_FIELD.findall(flat):
            if not (value.endswith(".short()") or value.startswith("diag::peer(")):
                yield f"{path}:{line_no}: log `{name}` by its short ID (`.short()`), not `{value}`"
        for name in DEVICE_SHORTHAND.findall(flat):
            yield f"{path}:{line_no}: log `{name}` by its short ID (`{name} = %{name}.short()`), not `%{name}`"


def main(argv):
    root = Path(argv[1]) if len(argv) > 1 else Path(".")
    found = []
    for top in DIRS:
        for path in sorted((root / top).rglob("*.rs")):
            if "target" in path.parts:
                continue
            source = without_tests(path.read_text(encoding="utf-8"))
            found += problems(path.relative_to(root).as_posix(), source)
    for p in found:
        print(p)
    if found:
        print(f"\n{len(found)} log line(s) could leak personal details; see .github/scripts/check_log_privacy.py")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
