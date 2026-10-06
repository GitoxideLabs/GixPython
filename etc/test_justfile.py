"""Check release recipe arguments without downloading or installing packages."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(shutil.which("just"), "requires just")
class JustfileTests(unittest.TestCase):
    def test_release_selection_and_python_arguments(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as directory:
            stub = Path(directory) / "uv"
            # Use the selected Python to record uv's arguments instead of running it.
            stub.write_text('#!/bin/sh\nexec "$PYTHON" -c \'import json, sys; print(json.dumps(sys.argv[1:]))\' "$@"\n')
            stub.chmod(0o755)
            env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"], PYTHON=sys.executable,
                       PYTHONPATH=str(root / "python"))
            for arguments, package, python_args in (
                ([], "GixPython", ["-i", "-c", 'import gix; print("GixPython", gix.__version__)']),
                (["0.1.0", "-c", "import gix; print(gix.__version__)"], "GixPython==0.1.0",
                 ["-c", "import gix; print(gix.__version__)"]),
                (["latest", "script with spaces.py", "argument with spaces"], "GixPython",
                 ["script with spaces.py", "argument with spaces"]),
            ):
                with self.subTest(arguments=arguments):
                    result = subprocess.run(["just", "--justfile", str(root / "justfile"), "run-release", *arguments],
                                            cwd=root, env=env, check=True, capture_output=True, text=True)
                    command = json.loads(result.stdout)
                    self.assertEqual(command, ["run", "--no-project", "--isolated", "--no-python-downloads",
                                               "--python", sys.executable, "--no-build", "--upgrade-package",
                                               "GixPython", "--with", package, "python", "-I", *python_args])


if __name__ == "__main__":
    unittest.main()
