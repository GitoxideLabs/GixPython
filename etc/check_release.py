"""Validate a release tag and the complete macOS distribution set locally."""

import argparse
from email.parser import BytesParser
from pathlib import Path
import tarfile
import tomllib
import zipfile

from check_artifact import check_sdist, check_wheel, require


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", help="Git release tag, for example v0.1.0")
    parser.add_argument("--dist", type=Path, help="Directory containing four wheels and one source archive")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    require(args.tag is None or args.tag == "v" + version, "release tag does not match Cargo.toml")
    require("\n## " + version + " (" in (root / "CHANGELOG.md").read_text(), "release changelog entry missing")
    if args.dist is not None:
        expected = {f"gixpython-{version}-{abi}-macosx_11_0_{arch}.whl"
                    for abi in ("cp311-abi3", "cp314-cp314t") for arch in ("arm64", "x86_64")}
        expected.add(f"gixpython-{version}.tar.gz")
        require({path.name for path in args.dist.iterdir()} == expected,
                "release must contain exactly four macOS wheels and one matching source archive")
        stubs = {path.name for path in (root / "python/gix").glob("*.pyi")}
        for path in sorted(args.dist.iterdir()):
            if path.suffix == ".whl":
                with zipfile.ZipFile(path) as archive:
                    metadata_name, = [name for name in archive.namelist() if name.endswith(".dist-info/METADATA")]
                    metadata = BytesParser().parsebytes(archive.read(metadata_name))
                    wheel = BytesParser().parsebytes(archive.read(metadata_name.removesuffix("METADATA") + "WHEEL"))
                require(metadata["Version"] == version, "wheel metadata version does not match release")
                tag = path.name.removesuffix(".whl").split("-", 2)[2]
                require(wheel.get_all("Tag") == [tag], "wheel ABI/platform does not match release filename")
                check_wheel(path, stubs)
            else:
                with tarfile.open(path) as archive:
                    metadata_name, = [name for name in archive.getnames() if name.count("/") == 1 and name.endswith("/PKG-INFO")]
                    metadata = BytesParser().parsebytes(archive.extractfile(metadata_name).read())
                require(metadata["Version"] == version, "source metadata version does not match release")
                check_sdist(path, stubs)
    print(f"Validated GixPython {version}" + (" release artifacts" if args.dist else " release metadata"))


if __name__ == "__main__":
    main()
