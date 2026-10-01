"""Build and load the extension locally, without accessing a Python index."""

import argparse
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
    command = ["cargo", "build", "--locked"]
    if args.release:
        command.append("--release")
    if args.no_default_features:
        command.append("--no-default-features")
    if args.features:
        command.extend(["--features", args.features])
    env = dict(os.environ, PYO3_PYTHON=sys.executable, PYO3_BUILD_EXTENSION_MODULE="1")
    subprocess.run(command, cwd=root, env=env, check=True)
    target = Path(env.get("CARGO_TARGET_DIR", root / "target"))
    profile = "release" if args.release else "debug"
    filename = "_gix.dll" if sys.platform == "win32" else "lib_gix.dylib" if sys.platform == "darwin" else "lib_gix.so"
    source = target / profile / filename
    destination = root / "python" / "gix" / ("_gix" + sysconfig.get_config_var("EXT_SUFFIX"))
    shutil.copy2(source, destination)
    print(destination)


if __name__ == "__main__":
    main()
