#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Assemble an UNSIGNED NectarLink Camera Extension in a disposable app.

Does not sign, install, activate, access an account, or download a helper.
See docs/development/macos-camera.md for the separate qualification gates.
"""
import argparse
import plistlib
import re
import subprocess
from pathlib import Path

HOST = "app.nectarlink.desktop"
EXTENSION = HOST + ".CameraExtension"
EXECUTABLE = "NectarLinkCameraExtension"


def metadata(team, version, build):
    if not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("Expected a qualified signing team ID")
    if not re.fullmatch(r"\d+\.\d+(?:\.\d+)?", version) or not re.fullmatch(r"\d+", build):
        raise ValueError("Expected numeric version/build")
    return dict(
        CFBundleIdentifier=EXTENSION,
        CFBundleExecutable=EXECUTABLE,
        CFBundleName="NectarLink Camera",
        CFBundlePackageType="SYSX",
        CFBundleShortVersionString=version,
        CFBundleVersion=build,
        LSMinimumSystemVersion="12.3",
        CMIOExtension={"CMIOExtensionMachServiceName": team + "." + EXTENSION},
        NSSystemExtensionUsageDescription="Use your approved phone camera in video apps.",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--candidate-app", type=Path, required=True)
    parser.add_argument("--team-id", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--build", required=True)
    parser.add_argument("--arch", choices=["arm64", "x86_64"], required=True)
    args = parser.parse_args()
    info = metadata(args.team_id, args.version, args.build)
    source = args.source_root.resolve(strict=True)
    candidate = args.candidate_app.resolve(strict=True)
    if candidate.suffix != ".app" or candidate == source or source in candidate.parents:
        parser.error("Use a disposable existing .app outside the source checkout")
    contents = candidate / "Contents"
    with (contents / "Info.plist").open("rb") as file:
        host = plistlib.load(file)
    if host.get("CFBundleIdentifier") != HOST:
        parser.error("Candidate must use the reviewed NectarLink host identifier")
    bundle = contents / "Library/SystemExtensions" / (EXTENSION + ".systemextension")
    # Refuse overwriting any installed/previous payload, including a symlink.
    if bundle.exists() or bundle.is_symlink():
        parser.error("Refusing to overwrite an existing extension payload")
    executable = bundle / "Contents/MacOS" / EXECUTABLE
    executable.parent.mkdir(parents=True)
    subprocess.run([
        "xcrun", "--sdk", "macosx", "clang", "-fobjc-arc", "-O2", "-arch", args.arch,
        "-mmacosx-version-min=12.3",
        str(source / "desktop/app/src/mac/camera_extension/NectarLinkCameraExtension.m"),
        "-framework", "Foundation", "-framework", "CoreMediaIO", "-framework", "CoreMedia",
        "-framework", "CoreVideo", "-framework", "Security", "-o", str(executable),
    ], check=True)
    with (bundle / "Contents/Info.plist").open("wb") as file:
        plistlib.dump(info, file)
    print("UNSIGNED payload assembled. Qualify profiles/entitlements and sign extension then host before activation.")


if __name__ == "__main__":
    main()
