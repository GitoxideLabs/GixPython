"""Build and load the extension locally, without accessing a Python index."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import sysconfig


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--no-default-features", action="store_true")
    parser.add_argument("--features")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    command = ["cargo", "build", "--locked", "--message-format=json-render-diagnostics"]
    if args.release:
        command.append("--release")
    if args.no_default_features:
        command.append("--no-default-features")
    if args.features:
        command.extend(["--features", args.features])
    env = dict(os.environ, PYO3_PYTHON=sys.executable, PYO3_BUILD_EXTENSION_MODULE="1")
    result = subprocess.run(command, cwd=root, env=env, stdout=subprocess.PIPE, text=True)
    source = None
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-message":
            diagnostic = message["message"].get("rendered")
            if diagnostic:
                print(diagnostic, file=sys.stderr, end="")
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "_gix":
            source = next((Path(name) for name in message["filenames"]
                           if Path(name).suffix in {".dll", ".dylib", ".so"}), source)
    result.check_returncode()
    if source is None:
        raise RuntimeError("Cargo did not produce the _gix extension library")
    destination = root / "python" / "gix" / ("_gix" + sysconfig.get_config_var("EXT_SUFFIX"))
    shutil.copy2(source, destination)
    print(destination)


if __name__ == "__main__":
    main()
