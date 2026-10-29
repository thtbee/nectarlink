#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline assembly tests. Mock compiler invocation; never run an SDK or signer."""
import importlib.util
import plistlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "camera_recipe", Path(__file__).with_name("build-native-camera-extension.py")
)
recipe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(recipe)


class RecipeTests(unittest.TestCase):
    def test_assembles_unsigned_native_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            app = root / "NectarLink.app"
            (app / "Contents").mkdir(parents=True)
            info = app / "Contents/Info.plist"
            host = {"CFBundleIdentifier": recipe.HOST, "Untouched": "preserved"}
            info.write_bytes(plistlib.dumps(host))
            args = ["recipe", "--source-root", str(source), "--candidate-app", str(app),
                    "--team-id", "ABCDEFGHIJ", "--version", "0.1.4", "--build", "1", "--arch", "arm64"]
            with patch.object(sys, "argv", args), patch.object(recipe.subprocess, "run") as run:
                recipe.main()
            command = run.call_args.args[0]
            self.assertEqual(command[:4], ["xcrun", "--sdk", "macosx", "clang"])
            self.assertNotIn("codesign", command)
            output = app / "Contents/Library/SystemExtensions" / (recipe.EXTENSION + ".systemextension")
            payload = plistlib.loads((output / "Contents/Info.plist").read_bytes())
            self.assertEqual(payload["CFBundleIdentifier"], recipe.EXTENSION)
            self.assertEqual(payload["CMIOExtension"]["CMIOExtensionMachServiceName"], "ABCDEFGHIJ." + recipe.EXTENSION)
            self.assertEqual(plistlib.loads(info.read_bytes()), host)
            self.assertTrue(run.call_args.kwargs["check"])

    def test_refuses_another_host_before_compilation_or_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            app = root / "Other.app"
            (app / "Contents").mkdir(parents=True)
            info = app / "Contents/Info.plist"
            original = plistlib.dumps({"CFBundleIdentifier": "app.other"})
            info.write_bytes(original)
            args = ["recipe", "--source-root", str(source), "--candidate-app", str(app),
                    "--team-id", "ABCDEFGHIJ", "--version", "0.1.4", "--build", "1", "--arch", "arm64"]
            with patch.object(sys, "argv", args), patch.object(recipe.subprocess, "run") as run:
                with self.assertRaises(SystemExit):
                    recipe.main()
            run.assert_not_called()
            self.assertEqual(info.read_bytes(), original)
            self.assertFalse((app / "Contents/Library").exists())


if __name__ == "__main__":
    unittest.main()
