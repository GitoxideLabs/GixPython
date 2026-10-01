# Development

## Prerequisites

Use the Rust version declared in `Cargo.toml`, CPython 3.11 or newer, and the maturin version required by `pyproject.toml`. Free-threaded builds require CPython 3.14 or newer, as required by PyO3; CI checks 3.14t separately. The Rust dependencies must not compile C or C++ sources; a system linker and the normal Rust target libraries are still needed.

Git is a test reference implementation and fixture-creation tool, not a runtime implementation dependency. Tests must create disposable repositories and isolate inherited Git configuration and environment variables.

The current task is local only. Rust dependency downloads needed for builds are authorized, but PyPI interaction, account writes, pushes, PRs, and publication are not. Use cached dependencies when practical. For tools whose default cache is outside the writable workspace, choose a task-owned cache under `/private/tmp` or the repository.

## Native upstream prerequisite

Some integration work may require small changes in the prepared local checkout at `/Users/byron/dev/github.com/GitoxideLabs/gitoxide.pygix`. That checkout and branch are not part of a GixPython source distribution.

A local Cargo override is a development mechanism, not a release dependency. Keep machine-specific absolute paths out of committed package manifests. Record any necessary upstream change and its validation before claiming that an ordinary clean checkout or source distribution builds without it. A release artifact must be reproducible from its declared, available dependencies; an unpublished local patch does not satisfy this requirement.

Do not push or create a PR during the current local-only task. If later authorized, upstream contributions use the prepared branch, the `pr-from-session` workflow, and draft PRs for Byron to review.

## Validation

The local builder uses Cargo and the selected interpreter directly; it does not query a Python package index:

```sh
python3 etc/build.py
PYTHONPATH=python python3 -m unittest discover -s tests
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

The `justfile` provides the same commands. Select an installed free-threaded interpreter in place of `python3` to test that ABI. Do not use blanket `--all-features` checks when feature choices are mutually exclusive.

Each new binding should have a focused Python integration check against a disposable repository. Verify errors as well as success, both supported hash kinds, and any meaningful reduced feature configuration. Compare with Git when it provides an appropriate reference result.

Threading checks share repository handles between Python threads and verify actual work with the GIL disabled. Lazy-API checks consume only a prefix and confirm that later results have not been eagerly produced. Long-running operations expose pollable `Progress.snapshot()` and `CancellationToken`; they must propagate Python interruption and release resources when stopped.

Reference-update tests retain an old target, change the reference independently, and verify that the stale update fails. Expected targets must never be re-resolved at mutation time. Preserve the engine's documented transaction guarantees; do not claim stronger atomicity.

## API maintenance

Keep native names and semantics. For example, Rust `Repository::commit()` creates a commit; Python must not reuse that name for a revision lookup. Native object-ID arguments may accept `str`/`bytes` revspecs through a shared private converter, without automatic peeling. Expected old targets in reference comparisons remain captured literal values.

Keep Python type information synchronized with bindings. Distinguish byte content from text, missing values from errors, and lazily yielded results from operations that inherently compute complete outcomes. Document compile-time capability choices without claiming that every enabled Rust feature is already bound.

## CI and artifacts

The GitHub workflows are configuration for future repository checks. They use pinned actions, minimal token permissions, and checkouts without persisted credentials. A stable `Tests pass` check collects the required CI results.

The artifact workflow builds distribution candidates for review. It does not upload to PyPI, create a GitHub release, or change package/account settings. Ordinary CPython and free-threaded CPython require their respective ABI configurations. Its initial Linux wheels target the build host (`linux_*`); they are not certified manylinux or musllinux wheels. Test an installed wheel and a source-distribution build, and add the appropriate Linux compatibility environment, before describing these candidates as release-ready.
