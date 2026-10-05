# GixPython project instructions

## Product and API

- The repository is `GitoxideLabs/GixPython`, the Python distribution is `GixPython`, and the import is `gix`.
- Build a complete PyO3 project exposing the Git capabilities available through the native `gix` crate. Keep an honest record of implemented and unsupported capabilities in the documentation.
- Preserve existing `gix::Repository` API names and semantics. Do not invent Git convenience methods or rename operations. A private argument converter may resolve `str`/`bytes` revspecs for native object-ID arguments, without implicit peeling. In particular, `Repository.commit()` creates a commit; it is not a lookup helper.
- Bind native operations instead of reimplementing Git or invoking the Git executable as an implementation fallback. Native configured helpers, such as SSH, credential helpers, filters, and signing tools, are distinct from a Git fallback.
- Keep lazy APIs lazy. Do not collect native iterators into Python lists or preload object contents that the caller has not requested. Document native operations that inherently compute a complete result.
- Preserve Git's byte-oriented paths, names, and contents. Provide explicit text conversions without silently discarding bytes. Preserve error causes and do not suppress corruption as absence.
- Preserve reference-update constraints. Compare-and-swap expected targets are captured literal values; never dynamically re-resolve them.

## Runtime and builds

- The first release supports macOS 11+ on Apple Silicon and Intel only. Linux and Windows CI are portability checks, not release support. Publish four macOS wheels (two architectures, ordinary and free-threaded CPython) and a source distribution.
- Support ordinary CPython 3.11 and newer with stable-ABI wheels. Free-threaded builds require CPython 3.14 or newer, as required by PyO3, and need their own compatible wheels and tests.
- Keep the Rust side thread-safe and parallel. Use `gix::ThreadSafeRepository` with operation-local native handles rather than serializing every operation behind a repository-wide lock.
- Detach from Python during native work. Never rely on the GIL for synchronization, and do not mark repository wrappers `unsendable`.
- Declare free-threading support only when the implementation satisfies it. Test a shared repository across threads with the GIL disabled.
- All supported configurations must build without compiling C or C++ dependencies. Use pure-Rust hashing/compression and reqwest/rustls with Graviola; do not accidentally enable a C-backed TLS provider through default dependency features.
- Default builds include both SHA-1 and SHA-256 and broad native capability groups. Keep important build choices explicit and test the supported reduced configurations.
- Follow current PyO3 APIs and ownership rules. Avoid `unsafe` and production `unwrap()`; justify any unavoidable exception locally.
- Progress uses a pollable `Progress.snapshot()` API and interruption uses `CancellationToken` plus normal Python exceptions. Do not add callbacks or install process-wide signal handlers. Do not call Python while holding native state locks.

## Ownership, review, and project files

- Use `MIT OR Apache-2.0`, with Sebastian Thiel as package author and copyright holder. Include both license texts in distributions.
- Divide work into meaningful, independently buildable and testable commits. Include documentation and relevant regression checks with the behavior they describe.
- Follow Gitoxide's purposeful conventional commits: use `feat:`/`fix:` for user-visible changes and plain descriptive subjects for maintenance. Use explicit Codex authorship for agent-created commits; package authorship is separate.
- Keep packaging, type information, examples, contributor guidance, CI, security policy, and artifact-building configuration part of the project.
- Tests use disposable repositories and isolated Git configuration/environment. Never mutate the developer's checkout, another worktree, or shared Git metadata as a fixture.
- The implementation sequence and persistent design decisions are in [docs/implementation-plan.md](docs/implementation-plan.md). Update its status as work lands; do not present planned coverage as completed.

## Current session boundary

- Work locally only: no PyPI interaction, account writes, push, publication, remote release, or pull request unless the user explicitly changes this boundary. Rust dependency downloads needed to build this project are authorized.
- The user subsequently authorized GitHub CI setup for this repository: push the committed project and CI fixes to `origin/main`, configure Actions, and verify runs on main and PRs. This does not authorize package publication or changes to the upstream Gitoxide remote.
- First-release preparation is authorized locally: metadata, Gitoxide-style changelog, macOS artifacts, and a manual Trusted Publishing workflow. PyPI account interaction and publication still require explicit authorization.
- Small necessary upstream changes may be prepared locally in `/Users/byron/dev/github.com/GitoxideLabs/gitoxide.pygix`. Preserve that prepared branch and unrelated changes. Inspect its applicable instructions and Tix state before changing history.
- Earlier permission to push upstream changes or create a draft PR is superseded by the local-only instruction. If later reauthorized, use `pr-from-session`, target `origin`, disclose Codex authorship, and create a draft PR only.
- Use available local caches when practical. Do not use a Python package index to install missing build tools; report missing prerequisites accurately.

`agents.md` is the single source of these instructions. `AGENTS.override.md` is only the automatic loader; do not add a competing case-only `AGENTS.md` file.
