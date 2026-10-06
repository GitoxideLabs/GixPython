# Local validation

## Local Gitoxide development builds (2026-10-06)

On Apple Silicon macOS, Cargo 1.99.0 resolved all 60 Gitoxide crates directly
from the selected checkout, including its three upstream fixes. Offline
compilation passed with C/C++ compilers disabled, preserving the committed
`Cargo.toml` and `Cargo.lock`. The development lockfile remained in ignored
`.cache/gitoxide-local/`. No persistent Cargo configuration was created.

Local CPython 3.14.7 passed 105 integration tests with two expected skips.
Local free-threaded CPython 3.14.7 passed the same suite with the GIL disabled
and one expected skip. Both exported checkout HEAD through `__gix_revision__`.
Forcing packaged mode with an invalid local-path environment value restored
the pinned revision and passed 105 tests with two expected skips.

The sdist had no developer dependency paths or local cache files. An ordinary
ABI3 macOS wheel built offline from its independently extracted sources,
with C/C++ compilers disabled and an invalid local-path setting, passed artifact
validation and all 105 installed-package tests (two expected skips). This
candidate is under `dist/local-gitoxide-validation/`; it is a development-profile
validation artifact, not a release candidate.

Five build-helper checks cover selection precedence, forced packaged mode,
invalid paths/provenance, the Cargo version gate, lockfile preservation,
source patching after a crates.io migration, and failures without fallback.
Actionlint and offline pedantic zizmor passed for the updated CI workflow.
Rust formatting, clippy with warnings denied, and all seven Rust tests passed.
Loopback network fixtures ran with sandbox access; the incomplete older cached
free-threaded interpreter was replaced for these checks by a task-owned binary
download from python-build-standalone, without using a Python package index.

## First release preparation (2026-10-05)

Version 0.1.0 now targets macOS 11+ on Apple Silicon and Intel. Updated
Apple Silicon wheels were built from the source archive using maturin 1.15.0
and Rust 1.99.0, without accessing a Python package index. Both installed
wheels passed all 105 integration tests: ordinary CPython 3.14.7 had two
expected skips; free-threaded CPython 3.14.7 with `PYTHON_GIL=0` had one.

Release metadata, source contents, licenses, type stubs, and both wheel
contents passed local validation. Actionlint 1.7.12 and offline pedantic
zizmor 1.30.1 passed; the same-commit reusable workflow syntax has one
documented compatibility suppression. Three regression checks verify the
release tag, artifact completeness, and wheel metadata version gates.

Current candidates and checksums are in `dist/0.1.0-macos-candidate/`.
Intel wheels and the complete five-artifact release set await the configured
GitHub workflow run. No PyPI account setup, tag, upload, or publication was
performed. See [RELEASING.md](../RELEASING.md) for the exact setup and commands.

## Initial implementation validation (2026-10-01)

These checks ran on macOS with Apple Silicon on 2026-10-01, using Rust 1.98.1
and maturin 1.15.0. They cover the final native implementation, including
configuration destruction/retry and clone revision-validation fixes.

## Installed distribution checks

The source archive was extracted and compared with the checkout, with only
maturin's expected Cargo manifest normalization differing. Optimized wheels
were built from that extracted archive. Tests run from the archive's tests and
fixtures against installations in disposable environments, without importing
the checkout's extension or accessing a Python package index.

| Release artifact | Interpreter | Full integration suite |
| --- | --- | --- |
| `gixpython-0.1.0-cp311-abi3-macosx_11_0_arm64.whl` | CPython 3.12.14 | 105 tests, 2 expected skips, no failures. |
| The same ABI3 wheel | CPython 3.14.7 | 105 tests, 2 expected skips, no failures. |
| `gixpython-0.1.0-cp314-cp314t-macosx_11_0_arm64.whl` | CPython 3.14.7 free-threaded, `PYTHON_GIL=0` | 105 tests, 1 expected skip, no failures. |

The ordinary-interpreter skips are the free-threaded-only import check and the
non-UTF-8 filename fixture rejected by this filesystem. The suite also verifies
that public runtime exports, members, properties, parameter names/kinds, and
defaults match the shipped Python type stubs.

The free-threaded suite verifies that importing `gix` leaves the GIL disabled;
only the filesystem fixture is skipped. Both wheels use the release profile.
The archive/wheel validator confirms vendored patch sources, package metadata,
dual-license files, extension modules, and type information. Local candidates
and their SHA-256 checksums are in the ignored `dist/` directory. The final
source archive also includes the completed documentation; this documentation
refresh does not change either wheel's native or Python implementation.

## Native builds and reduced configurations

| Check | Result |
| --- | --- |
| `cargo test --locked --lib` | All 7 Rust tests passed. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed. |
| Clippy without defaults, with `sha1,sha256,revision,index` | Passed, including all targets. |
| Clippy without defaults, with `sha1,sha256,http` | Passed, including all targets. |
| `cargo fmt --all -- --check` and `git diff --check` | Passed. |
| SHA-1-only build, full Python discovery on CPython 3.14.7 | 105 tests, 75 expected skips, no failures. |
| SHA-256-only build, full Python discovery on CPython 3.14.7 | 105 tests, 75 expected skips, no failures. |

The hash-only skips cover disabled capabilities, the free-threaded-only check,
and a filesystem that rejects the non-UTF-8 filename fixture. Native parallelism
remains enabled in both builds.

Default dependencies were compiled in a fresh target directory with
`CC=/usr/bin/false CXX=/usr/bin/false`; the final source was rebuilt successfully
using the same disabled compilers. This verifies compilation without C or C++
sources on this target. The system linker is still required.

The repository and progress examples were also run locally. Regression checks
cover shared handles, concurrent index snapshots, lazy demand/cancellation,
configuration cleanup and retry, stale reference constraints, byte preservation,
native clone/fetch, and loopback HTTP/TLS configuration.

## Limits of this validation

Linux, Windows, CPython 3.11/3.13, and Rust 1.89 are configured in GitHub
workflows but were not executed locally. The lockfile's selected dependency
metadata was checked against the declared Rust minimum; that does not replace
an actual MSRV build. Initial Linux artifact candidates use host-platform tags,
not manylinux or musllinux certification.

Free-threaded CPython 3.13 is unsupported by PyO3 0.29.3; the free-threaded
matrix starts at CPython 3.14. Ordinary CPython 3.13 remains supported.

The initial implementation was validated locally before CI was activated.
Nothing was published or created on PyPI. Three necessary upstream fixes remain
local and are included as vendored
source patches; their provenance is recorded in [DEVELOPMENT.md](../DEVELOPMENT.md).
The [API coverage record](api-coverage.md) lists unsupported native overloads,
callback interfaces, and operations unavailable in the pinned Gitoxide engine.
