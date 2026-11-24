# Local validation

## Version 0.4.0 preparation (2026-11-24)

The release metadata check accepts `v0.4.0`. The nine helper tests, Rust
formatting, and locked offline Cargo metadata checks pass. On Apple Silicon,
the packaged-dependency extension builds offline with C/C++ compilers disabled
and reports `0.4.0`. The CPython 3.12.14 integration suite passed 111 tests
with two skips. The Release workflow covers the four macOS wheels and the
source archive.

## Version 0.3.0 preparation (2026-10-24)

The manifest, root lockfile package, locked offline Cargo metadata, and
rebuilt Python runtime agree on `0.3.0`. The release gate accepts `v0.3.0`,
all nine helper checks pass, and formatting and whitespace checks pass.
The dated changelog covers the crates.io `gix 0.90.0` integration and
unconditional Sirocco selection while retaining `Unreleased` for future work.
The release guide uses the new version and preserves existing tags.

The default ABI3 extension builds offline with C/C++ compilers disabled on
Apple Silicon. CPython 3.14.7 passes all 110 discovered integration tests
with two expected skips, including localhost HTTP/TLS fixtures. Packaged
Gitoxide provenance remains `dd39fb66e3b1551c9fe849043e39b357a9799ca1`.
Dependency and reduced-build validation is recorded below.

No release artifacts, free-threaded or Intel builds, remote CI, tag, push,
or publication were performed. Maturin and a working free-threaded
interpreter remain unavailable locally; the Release workflow must validate
the complete four-wheel and source-archive set before publication.

## Crates.io `gix 0.90.0` integration (2026-10-24)

Packaged dependencies resolve entirely from crates.io: `gix 0.90.0`,
`gix-archive 0.39.0`, and `gix-testtools 0.22.0`. The lockfile preserves
unrelated dependency versions. Default and hash-only dependency metadata
enable `gix/zlib-sirocco` and `gix-zlib/sirocco`; all resolved dependency
MSRVs fit Rust 1.89. Packaged provenance uses the published `gix` crate's
source revision `dd39fb66e3b1551c9fe849043e39b357a9799ca1`.

On Apple Silicon macOS with Rust/Cargo 1.99.1 and CPython 3.14.7, the
default ABI3 build passes all 110 discovered integration tests with two
expected skips. The sandbox blocks loopback server binding; the full suite
passes with localhost access allowed. SHA-1-only and SHA-256-only ABI3
builds each pass 110 discovered tests with 75 expected skips. All builds
disable C/C++ compilers, including the default dependency build from scratch.
The published fixture symlink preflight passes a disposable scripted fixture
check, including rejection of a missing preflight result.

Default clippy and reduced clippy for `sha1,sha256,revision,index` and
`sha1,sha256,http` pass with warnings denied. Seven Rust unit tests, nine
build/release/justfile helper checks, formatting, and whitespace checks pass.
The local build selector's helper check covers all three crates.io patches.
The checkout's extension is restored to the default ABI3 configuration.

Maturin and a working free-threaded interpreter are unavailable locally, so
no wheel/sdist or free-threaded build was performed. Actual Rust 1.89,
Intel/cross-platform builds, a real local Gitoxide checkout build, remote CI,
push, and publication were not performed.

## Version 0.2.0 preparation (2026-10-14)

The Cargo manifest, root lockfile package, locked offline Cargo metadata,
and Python package version assertion agree on `0.2.0`. The release metadata
gate accepts `v0.2.0`; all nine build/release/justfile helper checks pass.
The dated changelog consolidates changes since `v0.1.0`, and the maintainer
instructions use the new version while preserving the existing tag.

The packaged extension builds offline on Apple Silicon with C/C++ compilers
disabled. CPython 3.14.7 passes all 110 integration tests with two expected
skips. The initial sandboxed run could not bind loopback HTTP/TLS servers;
the full suite passes when localhost access is allowed. All seven Rust unit
tests, Rust formatting, and whitespace checks pass. The runtime reports
`0.2.0` and the packaged Gitoxide revision
`09f85be4e3fda42f3e8fd19c53a63758fdf6fb3d`.

No `0.2.0` release artifacts, free-threaded or Intel builds, remote CI, tag,
push, or publication were performed. Maturin is unavailable locally; the
manual Release workflow remains responsible for building and validating
the complete four-wheel and source-archive set before publication.

## Python fixture tools (2026-10-12)

The pinned `gix-testtools 0.22.0` is available as `gix.testtools` in default
and reduced builds. Python producers and post-processors retain native
`FixtureState`, cache versions, archives, and callback return values. Five
integration checks run in disposable child-process workspaces and cover
concurrent initialization, cross-process cache reuse, archive extraction,
script arguments and Git environment isolation, all bound post-processing
variants, writable copy/direct-generation cleanup, exception identity,
failed-fixture retry, invalid inputs, and Python signal-handler preservation.

On Apple Silicon macOS with Rust/Cargo 1.99.0, packaged CPython 3.14.7 passes
all 110 tests with two expected skips; free-threaded CPython 3.14.7 with
`PYTHON_GIL=0` passes all 110 with one skip. Both hash-only configurations
pass 110 discovered tests with 75 expected skips, including the fixture
checks. Default and reduced clippy (`sha1,sha256,revision,index`) pass with
warnings denied; seven Rust tests, nine helper checks, formatting, and
whitespace checks pass. Local dependency metadata resolves `gix-testtools`
from the selected checkout's `tests/tools` directory. Dependency metadata
fits Rust 1.89 without an actual MSRV run. Project builds disable C/C++
compilers; fixture archives use TAR without the C-backed XZ feature.

The ABI3 macOS wheel rebuilt from its sdist includes `gix.testtools` and
its stubs, and passes all 110 tests on CPython 3.14.7 with two expected
skips in an offline installation. Archive and wheel content checks pass.
Development-profile candidates are in `dist/testtools-validation/`.

The fixture API is a supported subset, documented in [the guide](testtools.md).
Read-only paths remain a caller contract rather than filesystem protection.
Callbacks must not recursively acquire the same fixture or change process
environment/working directory while fixture calls run concurrently.
No remote operations, publication, Intel/cross-platform builds, or actual
Rust 1.89 execution were performed.

## Public `main` integration without vendoring (2026-10-12)

On Apple Silicon macOS with Rust/Cargo 1.99.0, packaged builds resolve public
Gitoxide `main` revision `09f85be4e3fda42f3e8fd19c53a63758fdf6fb3d`, the merge
of Gitoxide PR #3054. Its native transport client configuration, raw reflog
messages, and literal submodule names replace all three vendored crates.
The latest published `gix 0.89.0` lacks those changes: an isolated build against
the registry release fails because `reqwest::Options::configure_client` is
absent, and source comparisons confirm the missing byte-preservation fixes.
The public Git pin remains necessary until a compatible release is published.

Packaged CPython 3.14.7 passes all 105 integration tests with two expected
skips. Free-threaded CPython 3.14.7 with `PYTHON_GIL=0` passes the same suite
with one expected skip. These exercise the former vendored fixes, concurrent
shared handles, lazy reads and cancellation, and loopback HTTP/TLS fixtures.
SHA-1-only and SHA-256-only builds each pass 105 discovered tests with 75
expected capability/platform skips. Worktree removal adopts the new native
default retry delay; existing removal lifecycle and safety checks pass.

Default clippy and reduced clippy for `sha1,sha256,revision,index` and
`sha1,sha256,http` pass with warnings denied. Seven Rust unit tests, nine
build/release/justfile helper checks, Rust formatting, and whitespace checks
pass. All 60 Gitoxide crates resolve to the selected local checkout without
packaged patches, and the local build passes all 105 integration tests on
CPython 3.14.7 with two expected skips. The tracked lockfile retains every unrelated registry
dependency; all dependency metadata fits Rust 1.89, without an actual MSRV run.
Project builds use disabled C/C++ compilers and cached dependencies after
fetching the public revision. A standalone free-threaded interpreter was
downloaded into the ignored worktree cache, without a Python package index.

An ABI3 macOS wheel rebuilt from the source archive passes all 105 tests
on CPython 3.12.14 with two expected skips, installed offline in a disposable
environment. Archive/wheel content validation and packaged provenance checks
pass. The archive contains no vendored crates, Cargo patches, developer paths,
local lockfiles, or compiled binaries. Development-profile candidates are in
`dist/main-reintegration-validation/`.

No upstream edits, pushes, publication, remote CI, Intel/cross-platform builds,
or actual Rust 1.89 execution were performed. The existing release tag is
unchanged; broader platform/release-profile checks remain configured in CI.

## Updated `pygix` integration (2026-10-08)

On Apple Silicon macOS with Rust/Cargo 1.99.0, local builds used `pygix` HEAD
`e4d2b313b840ea0e00453791882dbc6eb7c636f6`. Packaged builds resolved the
public base `1d7bac742f70b72ddda0c7294f4a97566b8db596`, plus three refreshed
vendored prerequisites. All bundled source files match the updated checkout.
The packaged lock retains the existing registry versions and unconditional
native parallelism through `gix-parallel`. Dependency metadata still fits the
Rust 1.89 minimum; that is not an actual MSRV build.

Both local and packaged modes passed all 105 integration tests on CPython
3.14.7 (two expected skips) and free-threaded CPython 3.14.7 with `PYTHON_GIL=0`
(one expected skip). Provenance reported the respective local/public pins.
These checks include concurrent shared handles, lazy reads and cancellation,
raw reflog/submodule names, linked worktrees, and loopback HTTP/TLS fixtures.
Focused assertions also cover fallible pathspec directory queries, including
empty and optional results. Packaged mode ignored an invalid local selection.

Rust formatting, default clippy with warnings denied, seven Rust unit tests,
and nine build/release/justfile helper checks passed. Reduced clippy passed for
`sha1,sha256,revision,index` and `sha1,sha256,http`. SHA-1-only and SHA-256-only
packaged builds each passed 105 discovered tests with 75 expected skips.
Builds ran offline after fetching the new public Git revision, with C/C++
compilers disabled and existing dependency caches reused.

An ordinary ABI3 macOS wheel built independently from the source archive,
installed offline into a disposable environment, passed all 105 tests (two
expected skips), content validation, and the public provenance check. Archive
checks found no developer dependency paths, unmerged revision pins, local
lockfiles, or compiled binaries. Candidates are in
`dist/pygix-reintegration-validation/` and use the development profile.

The first archive build reused stale Cargo build-script output because tar's
fixed source timestamps predated the shared cache. Refreshing the extracted
build input timestamps forced recompilation, after which the wheel reported
the new public revision. The validated wheel is this rebuilt artifact.
Whitespace checks passed outside `vendor/`; vendored upstream documentation
retains trailing whitespace to preserve exact source copies.

No upstream edits, pushes, publication, remote CI, Intel builds, or actual
Rust 1.89 execution were performed for this reintegration. Existing tags are
unchanged. Cross-platform and release-profile checks remain configured in CI.

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
