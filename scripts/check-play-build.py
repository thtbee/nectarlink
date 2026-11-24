#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Checks a Google Play build of the phone app (the `play` flavor) for what
Play's policies don't allow, so it can't slip back in unnoticed.

    python scripts/check-play-build.py <play APK> <aapt2>

The APK is the release APK of the `play` flavor (the bundle holds the same
manifest, code and libraries). Fails when:

- the merged manifest asks for a permission the Play build leaves out
  (src/play/AndroidManifest.xml and src/github/AndroidManifest.xml);
- the code still holds the self-updater's or the donation link's addresses;
- a 64-bit native library isn't aligned for 16 KB memory pages, which Play
  requires of new apps and updates.
"""

import re
import struct
import subprocess
import sys
import zipfile

REMOVED_PERMISSIONS = [
    "android.permission.REQUEST_INSTALL_PACKAGES",
    "android.permission.UPDATE_PACKAGES_WITHOUT_USER_ACTION",
    "android.permission.USE_FULL_SCREEN_INTENT",
    "android.permission.SYSTEM_ALERT_WINDOW",
    "android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS",
    "android.permission.MANAGE_EXTERNAL_STORAGE",
    "android.permission.QUERY_ALL_PACKAGES",
]

# Text the GitHub build's code has and the Play build's must not: the update
# feed and releases (self-update), and the donation page.
FORBIDDEN_IN_CODE = [
    b"releases/download/",
    b"api.github.com/repos/thtbee/nectarlink/releases",
    b"buymeacoffee.com",
    b"nightly.json",
]

PAGE_16K = 0x4000


def manifest_permissions(apk: str, aapt2: str) -> set[str]:
    tree = subprocess.run(
        [aapt2, "dump", "xmltree", "--file", "AndroidManifest.xml", apk],
        check=True, capture_output=True, text=True,
    ).stdout
    permissions = set()
    element = None
    for line in tree.splitlines():
        m = re.match(r"\s*E: (\S+)", line)
        if m:
            element = m.group(1)
            continue
        m = re.match(r'\s*A: (?:http://schemas.android.com/apk/res/android:)?name\(0x01010003\)="([^"]+)"', line)
        if m and element in ("uses-permission", "uses-permission-sdk-23"):
            permissions.add(m.group(1))
    return permissions


def load_aligned(data: bytes) -> list[str]:
    """The PT_LOAD segments of a 64-bit ELF not aligned to 16 KB, as text."""
    if data[:4] != b"\x7fELF" or data[4] != 2:
        return []
    little = data[5] == 1
    order = "<" if little else ">"
    phoff = struct.unpack_from(order + "Q", data, 0x20)[0]
    phentsize, phnum = struct.unpack_from(order + "HH", data, 0x36)
    bad = []
    for i in range(phnum):
        base = phoff + i * phentsize
        p_type = struct.unpack_from(order + "I", data, base)[0]
        if p_type != 1:  # PT_LOAD
            continue
        p_align = struct.unpack_from(order + "Q", data, base + 0x30)[0]
        if p_align < PAGE_16K:
            bad.append(f"LOAD #{i} aligned to {p_align:#x}")
    return bad


def data_offset(raw, info: zipfile.ZipInfo) -> int:
    """Where an entry's bytes start: after its local header, whose extra field
    (zipalign's padding) can differ from the central directory's."""
    raw.seek(info.header_offset + 26)
    name_len, extra_len = struct.unpack("<HH", raw.read(4))
    return info.header_offset + 30 + name_len + extra_len


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    apk, aapt2 = sys.argv[1], sys.argv[2]
    problems = []

    permissions = manifest_permissions(apk, aapt2)
    if not permissions:
        problems.append("read no permissions from the manifest: is this the right APK?")
    for permission in REMOVED_PERMISSIONS:
        if permission in permissions:
            problems.append(f"the manifest asks for {permission}")

    with open(apk, "rb") as raw, zipfile.ZipFile(apk) as z:
        dexes = [n for n in z.namelist() if re.fullmatch(r"classes\d*\.dex", n)]
        if not dexes:
            problems.append("no code (classes.dex) in the APK")
        for name in dexes:
            code = z.read(name)
            for text in FORBIDDEN_IN_CODE:
                if text in code:
                    problems.append(f"{name} has {text.decode()!r}")
        libs = [n for n in z.namelist() if n.startswith("lib/") and n.endswith(".so")]
        for name in libs:
            abi = name.split("/")[1]
            if abi not in ("arm64-v8a", "x86_64"):
                continue
            info = z.getinfo(name)
            for issue in load_aligned(z.read(name)):
                problems.append(f"{name}: {issue}")
            if info.compress_type == zipfile.ZIP_STORED and data_offset(raw, info) % PAGE_16K:
                problems.append(f"{name} isn't at a 16 KB boundary in the APK (zipalign -P 16)")

    print(f"{apk}: {len(permissions)} permissions, {len(dexes)} dex files, {len(libs)} native libraries")
    for problem in problems:
        print(f"error: {problem}")
    if problems:
        return 1
    print("The Play build is clean.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
