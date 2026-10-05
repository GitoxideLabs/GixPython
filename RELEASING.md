# Releasing GixPython

The first version is **0.1.0**, for **macOS 11+ on Apple Silicon and Intel**.
The release contains four wheels and one source archive:

| Architecture | Ordinary CPython | Free-threaded CPython |
| --- | --- | --- |
| Apple Silicon | `cp311-abi3`, CPython 3.11+ | `cp314-cp314t`, CPython 3.14 |
| Intel | `cp311-abi3`, CPython 3.11+ | `cp314-cp314t`, CPython 3.14 |

All wheels use the default `max-pure` features, including both hash kinds.
Linux and Windows are portability checks only; they have no supported release wheels.
Source builds require Rust 1.89+, a linker, and access to the pinned Rust dependencies.

## One-time maintainer setup

Use a PyPI account with two-factor authentication. Register a pending GitHub
Trusted Publisher with these exact values:

| Field | Value |
| --- | --- |
| PyPI project name | `GixPython` |
| GitHub owner | `GitoxideLabs` |
| GitHub repository | `GixPython` |
| Workflow filename | `release.yml` |
| GitHub environment | `pypi` |

The first successful upload creates the PyPI project. No API token is needed.
Create the GitHub environment `pypi`, require a maintainer's approval, and allow
deployment from release tags `v*`. Make sure the maintainer who starts the run
can also approve it, if they are the only reviewer. Confirm the package name is
available when registering; it has not been checked or reserved by this project.

Documentation links point to this GitHub repository. Make them publicly
accessible before publishing, either by making the repository public or by
hosting the linked documentation publicly and updating the URLs.

## Prepare and inspect

Keep the version in `Cargo.toml`, its root `Cargo.lock` package entry, and
`tests/test_package.py` aligned. Python distribution versions come from Cargo.
Use Gitoxide's changelog style: `## VERSION (YYYY-MM-DD)`, with `New Features`,
`Bug Fixes`, and `Other` sections when applicable. Set the date to the actual
release day before creating the tag.

Run CI on the final commit. Then start the **Release** workflow from that
commit's branch with **publish unchecked**, or use:

```sh
gh workflow run release.yml --repo GitoxideLabs/GixPython --ref main -f publish=false
```

Each macOS wheel is built from the source archive, checked, installed without
an index, and tested. The final validation requires exactly four matching macOS
wheels and one source archive. Download a successful run for inspection:

```sh
gh run download RUN_ID --repo GitoxideLabs/GixPython --dir dist/release --pattern 'wheel-*'
gh run download RUN_ID --repo GitoxideLabs/GixPython --dir dist/release --name source-distribution
```

The downloads are grouped by artifact name. Inspect their contents and keep
older development artifacts separate from the release set.

## Publish

After reviewing the tested commit, create an annotated tag `v0.1.0` and push
that tag to `origin`. Start **Release** again with that tag selected and
**publish checked**:

```sh
gh workflow run release.yml --repo GitoxideLabs/GixPython --ref v0.1.0 -f publish=true
```

The workflow checks that the tag matches the version, rebuilds and tests the
complete set, then waits for the `pypi` environment approval. Approve the job
after inspecting its artifacts. The official PyPI action uploads those exact
artifacts using Trusted Publishing and generates provenance attestations.
There is no automatic publish on branch pushes or tag creation.

Once uploaded, verify on both architectures and both interpreter kinds:

```sh
python -m venv .release-check
.release-check/bin/python -m pip install --only-binary=:all: GixPython==0.1.0
.release-check/bin/python -c 'import gix; print(gix.__version__, gix.build_features())'
```

For free-threaded Python, set `PYTHON_GIL=0` and verify importing `gix` leaves
`sys._is_gil_enabled()` false. An optional GitHub release can use the version's
changelog text. PyPI filenames cannot be replaced after upload; corrections
require a new version.
