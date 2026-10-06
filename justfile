python := env_var_or_default("PYTHON", "python3")
j := quote(just_executable())

# Show the available development commands.
[private]
default:
    @{{ j }} --list --unsorted

# Build for the selected installed Python without a Python package index.
[group('Development')]
[positional-arguments]
build *args:
    {{ quote(python) }} etc/build.py "$@"

# Run the Python integration checks against the local extension.
[group('Tests')]
test: build
    PYTHONPATH=python {{ quote(python) }} -m unittest discover -s tests

# Check Rust formatting and lints.
[group('Development')]
check:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings

# Format Rust sources and the justfile.
[group('Development')]
fmt:
    cargo fmt --all
    {{ j }} --fmt --unstable

# Build an optimized local extension.
[group('Development')]
release:
    {{ quote(python) }} etc/build.py --release

# Exercise the extension with an installed free-threaded Python.
[group('Tests')]
test-free-threaded interpreter:
    {{ quote(interpreter) }} etc/build.py
    PYTHONPATH=python PYTHON_GIL=0 {{ quote(interpreter) }} -m unittest discover -s tests

# Run Rust unit tests against the packaged dependencies.
[group('Tests')]
unit-tests:
    cargo test --locked --lib

# Check the build, release, and justfile helpers without building the extension.
[group('Tests')]
helper-tests:
    {{ quote(python) }} etc/test_build.py
    {{ quote(python) }} etc/test_check_release.py
    {{ quote(python) }} etc/test_justfile.py

# Build review candidates from a complete source archive; never publish them.
[group('Releases')]
[positional-arguments]
artifacts *args:
    maturin build --sdist --release --locked --out dist --interpreter {{ quote(python) }} "$@"
    {{ quote(python) }} etc/check_artifact.py dist/*.tar.gz dist/*.whl

# Explore a release in IPython with completion, or run Python arguments in an isolated uv virtualenv.
[group('Releases')]
[positional-arguments]
run-release version='latest' *args:
    version="$1"; shift; \
        package=GixPython; \
        if [ "$version" != latest ]; then package="GixPython==$version"; fi; \
        if [ "$#" -eq 0 ]; then \
            set -- --with ipython python -I -m IPython --quick -i -c 'import gix; print("GixPython", gix.__version__)'; \
        else set -- python -I "$@"; fi; \
        uv run --no-project --isolated --no-python-downloads --python {{ quote(python) }} \
            --no-build --upgrade-package GixPython --with "$package" "$@"
