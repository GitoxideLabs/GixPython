# Development

## Prerequisites

Use the Rust version declared in `Cargo.toml`, CPython 3.11 or newer, and the maturin version required by `pyproject.toml`. Free-threaded builds require CPython 3.14 or newer, as required by PyO3; CI checks 3.14t separately. The Rust dependencies must not compile C or C++ sources; a system linker and the normal Rust target libraries are still needed.

Git is a test reference implementation and fixture-creation tool, not a runtime implementation dependency. Tests must create disposable repositories and isolate inherited Git configuration and environment variables.

GitHub CI setup is authorized for this repository, including pushing the committed project and CI fixes to `origin/main`. Package publication, PyPI interaction, and changes to the upstream Gitoxide remote remain outside the task. Use cached dependencies when practical. For tools whose default cache is outside the writable workspace, choose a task-owned cache under `/private/tmp` or the repository.

## Native upstream prerequisite

The manifest pins Gitoxide revision `f819565c2c4c56619c4888acef6cf3b8144cbccb`. Three small fixes are included as source-preserving Cargo patches under `vendor/`:

| Crate | Local upstream commit | Reason |
| --- | --- | --- |
| `gix-transport` | `a13b2ced126803aab080a43dc89b9756fb9bfebd` | Per-client reqwest TLS configuration with cached/lazy initialization and redirect policy. |
| `gix-ref` | `97c08543a63252032f9baf7f8de89cd0996db8d9` | Preserve raw reflog message bytes. |
| `gix-submodule` | `fe215611d5960db7b4c534a6dba46b8b338fcf15` | Query submodule configuration using literal byte names. |

Each directory includes its licenses and `README.pygix.md` provenance. Cargo manifests reference the pinned Git source for sibling dependencies. The source distribution includes these patches and does not require the developer's sibling checkout or an unpublished remote commit. Remove each patch after updating to an upstream revision containing its fix.

The corresponding local upstream commits are in the prepared `gitoxide.pygix` checkout for Sebastian Thiel's review. Nothing was pushed. If later authorized, upstream contributions use `pr-from-session` and draft PRs only.

## Validation

See the [local validation record](docs/validation.md) for tested interpreters,
artifacts, feature selections, and the limits of the checks performed.

The local builder uses Cargo and the selected interpreter directly; it does not query a Python package index:

```sh
python3 etc/build.py
PYTHONPATH=python python3 -m unittest discover -s tests
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --lib
```

The `justfile` provides the same commands. Select an installed free-threaded interpreter in place of `python3` to test that ABI. Use `PYTHON_GIL=0` for free-threaded tests; the package test also verifies importing the extension keeps the GIL disabled.

Each new binding should have a focused Python integration check against a disposable repository. Verify errors as well as success, both supported hash kinds, and any meaningful reduced feature configuration. Compare with Git when it provides an appropriate reference result.

Threading checks share repository handles between Python threads and verify actual work with the GIL disabled. Lazy-API checks consume only a prefix and confirm that later results have not been eagerly produced. Long-running operations expose pollable `Progress.snapshot()` and `CancellationToken`; they must propagate Python interruption and release resources when stopped.

Reference-update tests retain an old target, change the reference independently, and verify that the stale update fails. Expected targets must never be re-resolved at mutation time. Preserve the engine's documented transaction guarantees; do not claim stronger atomicity.

## API maintenance

Keep native names and semantics. For example, Rust `Repository::commit()` creates a commit; Python must not reuse that name for a revision lookup. Native object-ID arguments may accept `str`/`bytes` revspecs through a shared private converter, without automatic peeling. Expected old targets in reference comparisons remain captured literal values.

Keep Python type information synchronized with bindings. Distinguish byte content from text, missing values from errors, and lazily yielded results from operations that inherently compute complete outcomes. Document compile-time capability choices without claiming that every enabled Rust feature is already bound.

## CI and artifacts

The GitHub workflows run on pushes to `main` and pull requests targeting `main`, with manual dispatch also available. They use pinned actions, minimal token permissions, and checkouts without persisted credentials. A stable `Tests pass` check collects the required CI results.

Private repositories run zizmor with workflow annotations instead of code-scanning uploads. CodeQL runs only for public repositories; enabling it here while private requires GitHub Advanced Security and an update to its job condition.

The artifact workflow builds distribution candidates for review. It does not upload to PyPI, create a GitHub release, or change package/account settings. Ordinary CPython and free-threaded CPython require their respective ABI configurations. Its initial Linux wheels target the build host (`linux_*`); they are not certified manylinux or musllinux wheels. Test an installed wheel and a source-distribution build, and add the appropriate Linux compatibility environment, before describing these candidates as release-ready.


## Feature selections

`max-pure` is the default aggregate. The independently selectable groups are declared in `Cargo.toml`; dependent features enable their required native capabilities automatically. Parallelism is unconditional. Select at least one of `sha1` and `sha256`. `signing` enables native signing configuration and execution without requiring network support. `http` adds the blocking HTTP transport, and `https` adds the pure Rust TLS provider. The maturin configuration enables `abi3` by default for ordinary CPython. PyO3 ignores that feature on free-threaded interpreters and builds their version-specific ABI instead.

```sh
python3 etc/build.py --no-default-features --features sha1
python3 etc/build.py --no-default-features --features sha256,revision,index
CC=/usr/bin/false CXX=/usr/bin/false CARGO_TARGET_DIR=target/no-c cargo build --locked
```

The last command performs a fresh build with C and C++ compilation disabled; it still uses the system linker. Graviola supports selected x86-64 and AArch64 CPU features; unsupported hardware receives a clear HTTPS configuration error before a provider operation. Builds without HTTPS do not need that provider.

## Local wheels

With maturin installed, these commands create local candidates without a Python index:

```sh
maturin build --locked --release --out dist --interpreter python3
maturin build --locked --release --out dist --interpreter /path/to/python3.14t
maturin sdist --out dist
```

Install a matching wheel into a disposable environment with `uv pip install --no-index --no-deps`, then run the test suite without `PYTHONPATH=python`. Rebuild the wheel from the unpacked sdist to verify its vendor/support files. The artifact workflow automates these checks. The stable ABI and free-threaded ABI are separate compatibility contracts; a wheel filename and successful import on one interpreter do not validate another ABI.
