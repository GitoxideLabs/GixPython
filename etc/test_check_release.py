"""Checks for release gates; these run without building or importing gix."""

from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
import zipfile


class ReleaseTests(unittest.TestCase):
    version = tomllib.loads((Path(__file__).resolve().parents[1] / "Cargo.toml").read_text())["package"]["version"]

    def run_check(self, *args):
        return subprocess.run([sys.executable, str(Path(__file__).with_name("check_release.py")), *args],
                              capture_output=True, text=True)

    def test_matching_and_mismatched_tag(self):
        self.assertEqual(self.run_check("--tag", "v" + self.version).returncode, 0)
        result = self.run_check("--tag", "v99.0.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("release tag does not match", result.stderr)

    def test_partial_set_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_check("--dist", directory)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly four macOS wheels", result.stderr)

    def test_renamed_wheel_cannot_hide_wrong_version(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for abi in ("cp311-abi3", "cp314-cp314t"):
                for arch in ("arm64", "x86_64"):
                    tag = f"{abi}-macosx_11_0_{arch}"
                    with zipfile.ZipFile(root / f"gixpython-{self.version}-{tag}.whl", "w") as archive:
                        archive.writestr(f"gixpython-{self.version}.dist-info/METADATA", "Name: GixPython\nVersion: 99.0.0\n")
                        archive.writestr(f"gixpython-{self.version}.dist-info/WHEEL", f"Tag: {tag}\n")
            (root / f"gixpython-{self.version}.tar.gz").touch()
            result = self.run_check("--dist", directory)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("wheel metadata version does not match", result.stderr)


if __name__ == "__main__":
    unittest.main()
