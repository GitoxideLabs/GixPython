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
