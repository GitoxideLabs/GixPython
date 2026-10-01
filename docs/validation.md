# Local validation

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

Linux, Windows, CPython 3.11/3.13/3.13t, and Rust 1.89 are configured in GitHub
workflows but were not executed locally. The lockfile's selected dependency
metadata was checked against the declared Rust minimum; that does not replace
an actual MSRV build. Initial Linux artifact candidates use host-platform tags,
not manylinux or musllinux certification.

No CI workflow was triggered, and nothing was pushed, published, or created on
PyPI. Three necessary upstream fixes remain local and are included as vendored
source patches; their provenance is recorded in [DEVELOPMENT.md](../DEVELOPMENT.md).
The [API coverage record](api-coverage.md) lists unsupported native overloads,
callback interfaces, and operations unavailable in the pinned Gitoxide engine.
