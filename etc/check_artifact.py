"""Check distribution contents without installing or contacting a package index."""
import argparse
from email.parser import BytesParser
from pathlib import Path
import tarfile
import tomllib
import zipfile


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check_sdist(path, expected_stubs):
    with tarfile.open(path) as archive:
        entries = {member.name: member for member in archive.getmembers()}
        roots = {name.split("/", 1)[0] for name in entries}
        require(len(roots) == 1, "sdist must have one top-level directory")
        prefix = roots.pop() + "/"
        names = {name.removeprefix(prefix) for name in entries}
        required = {"Cargo.toml", "Cargo.lock", "build.rs", "pyproject.toml", "src/lib.rs",
                    "README.md", "LICENSE-MIT", "LICENSE-APACHE", "python/gix/__init__.py",
                    "python/gix/py.typed", "etc/build.py", "etc/check_artifact.py", "tests/test_package.py"}
        required.update("python/gix/" + name for name in expected_stubs)
        manifest = tomllib.loads(archive.extractfile(prefix + "Cargo.toml").read().decode())
        for patches in manifest.get("patch", {}).values():
            for dependency in patches.values():
                if "path" not in dependency:
                    continue
                directory = dependency["path"]
                require(not Path(directory).is_absolute() and ".." not in Path(directory).parts,
                        "sdist patch dependencies must live inside the archive")
                required.update(directory + "/" + name for name in
                                ("Cargo.toml", "src/lib.rs", "LICENSE-MIT", "LICENSE-APACHE", "README.pygix.md"))
        require(required <= names, f"sdist missing: {sorted(required - names)}")
        require(not any(name.endswith((".so", ".pyd", ".dylib", ".pyc")) for name in names),
                "sdist contains compiled artifacts")


def check_wheel(path, expected_stubs):
    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        required = {"gix/__init__.py", "gix/py.typed"}
        required.update("gix/" + name for name in expected_stubs)
        require(required <= names, f"wheel missing: {sorted(required - names)}")
        metadata_paths = [name for name in names if name.endswith(".dist-info/METADATA")]
        require(len(metadata_paths) == 1, "wheel must have one metadata directory")
        metadata = BytesParser().parsebytes(archive.read(metadata_paths[0]))
        require(metadata["Name"] == "GixPython", "incorrect Python distribution name")
        require(metadata["License-Expression"] == "MIT OR Apache-2.0", "missing dual-license expression")
        for license_name in ("LICENSE-MIT", "LICENSE-APACHE"):
            require(any(name.endswith(".dist-info/licenses/" + license_name) for name in names),
                    f"wheel missing license metadata: {license_name}")
        binaries = [name for name in names if name.startswith("gix/_gix") and name.endswith((".so", ".pyd"))]
        require(len(binaries) == 1, f"expected one extension module, found {binaries}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", nargs="+", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    expected_stubs = {path.name for path in (root / "python/gix").glob("*.pyi")}
    for artifact in args.artifacts:
        if artifact.name.endswith(".tar.gz"):
            check_sdist(artifact, expected_stubs)
        elif artifact.suffix == ".whl":
            check_wheel(artifact, expected_stubs)
        else:
            parser.error(f"unsupported artifact: {artifact}")
        print(f"Verified {artifact}")


if __name__ == "__main__":
    main()
