python := env_var_or_default("PYTHON", "python3")

# Show the available development commands.
default:
    @just --list

# Build for the selected installed Python without a Python package index.
build *args:
    {{python}} etc/build.py {{args}}

# Run the Python integration checks against the local extension.
test: build
    PYTHONPATH=python {{python}} -m unittest discover -s tests

# Check Rust formatting and lints.
check:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Build an optimized local extension.
release:
    {{python}} etc/build.py --release

# Exercise the extension with an installed free-threaded Python.
test-free-threaded interpreter:
    {{interpreter}} etc/build.py
    PYTHONPATH=python PYTHON_GIL=0 {{interpreter}} -m unittest discover -s tests

# Build review candidates from a complete source archive; never publish them.
artifacts *args:
    #!/usr/bin/env bash
    set -euo pipefail
    abi_args=()
    if [[ "$({{python}} -c 'import sysconfig; print(bool(sysconfig.get_config_var("Py_GIL_DISABLED")))')" == False ]]; then
        abi_args+=(--features abi3)
    fi
    maturin build --sdist --release --locked --out dist --interpreter {{python}} "${abi_args[@]}" {{args}}
    {{python}} etc/check_artifact.py dist/*.tar.gz dist/*.whl
