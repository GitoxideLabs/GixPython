"""Local dependency selection checks, without compiling or importing gix."""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import build


class BuildTests(unittest.TestCase):
    def test_selection_and_build_options(self):
        for arguments, environment, expected in (
            ([], {}, None),
            ([], {"GIXPYTHON_GITOXIDE_PATH": "/env-checkout"}, "/env-checkout"),
            (["--gitoxide-path", "/flag-checkout"], {"GIXPYTHON_GITOXIDE_PATH": "/env-checkout"}, "/flag-checkout"),
            (["--packaged"], {"GIXPYTHON_GITOXIDE_PATH": "/invalid-checkout"}, None),
        ):
            with self.subTest(arguments=arguments, environment=environment), \
                    patch.dict(os.environ, environment, clear=True), \
                    patch.object(build, "local_cargo_config", return_value=["--config", "local=true"]) as config, \
                    patch.object(build.subprocess, "run") as run, \
                    patch.object(build.shutil, "copy2") as copy, \
                    contextlib.redirect_stdout(io.StringIO()):
                artifact = subprocess.CompletedProcess([], 0, stdout=json.dumps({
                    "reason": "compiler-artifact", "target": {"name": "_gix"},
                    "filenames": ["/custom-target/lib_gix.dylib"],
                }))
                revision = "a" * 40
                run.side_effect = ([subprocess.CompletedProcess([], 0, stdout=revision)] if expected else []) + [artifact]
                build.main(arguments + ["--release", "--no-default-features", "--features", "sha256"])
                command = run.call_args.args[0]
                self.assertIn("--release", command)
                self.assertIn("--no-default-features", command)
                self.assertEqual(command[-2:], ["--features", "sha256"])
                self.assertEqual("--locked" in command, expected is None)
                self.assertEqual(run.call_args.kwargs["env"]["PYO3_PYTHON"], build.sys.executable)
                self.assertEqual(run.call_args.kwargs["env"].get("GIXPYTHON_BUILD_GIX_REVISION"), revision if expected else None)
                copy.assert_called_once()
                self.assertEqual(copy.call_args.args[0], Path("/custom-target/lib_gix.dylib"))
                if expected is None:
                    config.assert_not_called()
                else:
                    self.assertEqual(config.call_args.args[1], Path(expected).resolve())

    def test_conflicting_flags_and_configuration_errors(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            build.main(["--gitoxide-path", ".", "--packaged"])
        self.assertEqual(error.exception.code, 2)
        with patch.object(build, "local_cargo_config", side_effect=ValueError("invalid checkout")), \
                patch.object(build.subprocess, "run") as run, \
                contextlib.redirect_stderr(io.StringIO()) as stderr, self.assertRaises(SystemExit):
            build.main(["--gitoxide-path", "."])
        self.assertIn("invalid checkout", stderr.getvalue())
        run.assert_not_called()

    def test_build_failure_does_not_fall_back_or_copy(self):
        with patch.dict(os.environ, {"GIXPYTHON_GITOXIDE_PATH": "/checkout"}, clear=True), \
                patch.object(build, "local_cargo_config", return_value=[]), \
                patch.object(build.subprocess, "run") as run, \
                patch.object(build.shutil, "copy2") as copy, \
                contextlib.redirect_stdout(io.StringIO()), self.assertRaises(subprocess.CalledProcessError):
            run.side_effect = [subprocess.CompletedProcess([], 0, stdout="a" * 40),
                               subprocess.CompletedProcess([], 1, stdout="")]
            build.main([])
        self.assertEqual(run.call_count, 2)  # Git provenance, then one build attempt.
        copy.assert_not_called()

    def test_invalid_git_head_and_packaged_provenance(self):
        with patch.object(build, "local_cargo_config", return_value=[]), \
                patch.object(build.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout="invalid")) as run, \
                contextlib.redirect_stderr(io.StringIO()) as stderr, self.assertRaises(SystemExit):
            build.main(["--gitoxide-path", "."])
        self.assertIn("valid Git HEAD", stderr.getvalue())
        run.assert_called_once()
        with patch.dict(os.environ, {"GIXPYTHON_GITOXIDE_PATH": "/checkout", "GIXPYTHON_BUILD_GIX_REVISION": "a" * 40}, clear=True), \
                patch.object(build.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, stdout="")) as run, \
                self.assertRaises(subprocess.CalledProcessError):
            build.main(["--packaged"])
        self.assertNotIn("GIXPYTHON_BUILD_GIX_REVISION", run.call_args.kwargs["env"])

    def test_local_configuration_and_lockfile(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(build.subprocess, "run") as run:
            root = Path(directory)
            checkout = root / "checkout with spaces"
            for name in ("gix", "gix-archive", "gix-transport", "gix-ref", "gix-submodule"):
                path = checkout / name / "Cargo.toml"
                path.parent.mkdir(parents=True)
                path.write_text(f'[package]\nname = "{name}"\n')
            manifest = '[dependencies]\ngix = { git = "https://example.com/gitoxide" }\ngix-archive = { git = "https://example.com/gitoxide" }\n'
            manifest += '[patch."https://example.com/gitoxide"]\n'
            manifest += '\n'.join(f'{name} = {{ path = "vendor/{name}" }}' for name in ("gix-transport", "gix-ref", "gix-submodule"))
            (root / "Cargo.toml").write_text(manifest)
            (root / "Cargo.lock").write_text("packaged lockfile\n")
            run.return_value.stdout = "cargo 1.97.0 (example)\n"
            config = build.local_cargo_config(root, checkout)
            lock = root / ".cache/gitoxide-local/Cargo.lock"
            self.assertEqual(lock.read_text(), "packaged lockfile\n")
            self.assertEqual(json.loads(config[1].split("=", 1)[1]), str(lock))
            self.assertEqual(len(config), 12)
            self.assertTrue(all('patch."https://example.com/gitoxide".' in value for value in config[3::2]))
            self.assertEqual({json.loads(value.split("=", 1)[1]) for value in config[3::2]},
                             {str(path.parent) for path in checkout.glob("*/Cargo.toml")})
            lock.write_text("updated development lockfile\n")
            build.local_cargo_config(root, checkout)
            self.assertEqual(lock.read_text(), "updated development lockfile\n")
            self.assertEqual((root / "Cargo.lock").read_text(), "packaged lockfile\n")
            self.assertEqual((root / "Cargo.toml").read_text(), manifest)
            self.assertFalse((root / ".cargo/config.toml").exists())
            # The same selector must work after the release dependencies move to crates.io.
            (root / "Cargo.toml").write_text('[dependencies]\ngix = { version = "1" }\ngix-archive = { version = "1" }\n')
            config = build.local_cargo_config(root, checkout)
            self.assertTrue(all('patch."crates-io".' in value for value in config[3::2]))
            lock.unlink()
            run.return_value.stdout = "cargo 1.96.0 (example)\n"
            with self.assertRaisesRegex(ValueError, "Cargo 1.97"):
                build.local_cargo_config(root, checkout)
            self.assertFalse(lock.exists())
            with self.assertRaisesRegex(ValueError, "gix/Cargo.toml"):
                build.local_cargo_config(root, root / "missing")


if __name__ == "__main__":
    unittest.main()
