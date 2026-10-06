"""Build and load the extension locally, without accessing a Python index."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import sysconfig
import tomllib


def local_cargo_config(root, checkout):
    """Override Gitoxide sources without editing the packaged manifest or lockfile."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    patches = {
        (dependency.get("git", "crates-io"), name)
        for name, dependency in manifest["dependencies"].items()
        if name == "gix" or name.startswith("gix-")
    }
    patches.update((source, name) for source, entries in manifest.get("patch", {}).items()
                   for name in entries if name == "gix" or name.startswith("gix-"))
    for _, name in sorted(patches):
        path = checkout / name / "Cargo.toml"
        if not path.is_file() or tomllib.loads(path.read_text()).get("package", {}).get("name") != name:
            raise ValueError(f"Gitoxide checkout must contain {name}/Cargo.toml for package {name}: {checkout}")
    result = subprocess.run(["cargo", "--version"], check=True, capture_output=True, text=True)
    version = re.match(r"cargo (\d+)\.(\d+)\.(\d+)", result.stdout)
    if version is None or tuple(map(int, version.groups())) < (1, 97, 0):
        raise ValueError("Local Gitoxide builds require Cargo 1.97+ for a separate development lockfile")
    lock = root / ".cache" / "gitoxide-local" / "Cargo.lock"
    lock.parent.mkdir(parents=True, exist_ok=True)
    if not lock.exists():
        shutil.copy2(root / "Cargo.lock", lock)
    config = ["--config", "resolver.lockfile-path=" + json.dumps(str(lock))]
    for source, name in sorted(patches):
        config.extend(["--config", f"patch.{json.dumps(source)}.{name}.path=" + json.dumps(str(checkout / name))])
    return config


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--no-default-features", action="store_true")
    parser.add_argument("--features")
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--gitoxide-path", type=Path, help="use this Gitoxide checkout instead of release dependencies")
    selection.add_argument("--packaged", action="store_true", help="use release dependencies, ignoring GIXPYTHON_GITOXIDE_PATH")
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[1]
    command = ["cargo", "build", "--message-format=json-render-diagnostics"]
    checkout = args.gitoxide_path
    if checkout is None and not args.packaged:
        checkout = os.environ.get("GIXPYTHON_GITOXIDE_PATH")
    if checkout:
        checkout = Path(checkout).expanduser().resolve()
        try:
            command.extend(local_cargo_config(root, checkout))
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            parser.error(str(error))
        print(f"Building against local Gitoxide: {checkout} (including uncommitted edits)", flush=True)
    else:
        command.append("--locked")
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
