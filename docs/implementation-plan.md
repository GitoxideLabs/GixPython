# Implementation plan

The steps below describe the complete intended project. The implementation is split into reviewable local commits with type information, documentation, and runnable checks. The API matrix records specific coverage and deliberate limits.

| Step | Capability | Status |
| --- | --- | --- |
| 1 | Package/runtime foundation: GixPython distribution, `gix` import, PyO3 ABI configuration, licenses, persistent instructions, and baseline project files. | Implemented |
| 2 | Repository discovery/open/init, thread-safe handles, hash kinds/object IDs, raw objects, native configuration and isolated open options. | Implemented |
| 3 | Structured objects: blobs, trees and entries, commits, tags, object creation/editing, and native signing/verification. | Implemented |
| 4 | References and configuration: HEAD, branches, namespaces, reflogs, captured-value updates, transactions, configuration reads/edits, and identities. | Implemented |
| 5 | Revision parsing and traversal: native revision specifications, history, merge bases, descriptions, shallow state, commit graphs, and mailmaps. | Implemented |
| 6 | Index/path facilities: index reads/edits/writes, tree conversion, attributes, excludes, pathspecs, filters, directory walking, and submodule inspection. | Implemented |
| 7 | Status and diff: worktree/index/tree comparisons, native diff options, rewrite detection, hunks, statistics, and binary handling. | Implemented |
| 8 | Blame and notes: native blame options/results and notes lookup, replacement, removal, and ref selection. | Implemented |
| 9 | Merge: blobs, trees, commits, virtual merge bases, native conflict data, and resolution controls. | Implemented |
| 10 | Worktrees and exports: native worktree lifecycle, checkout, tree streams, and supported archive formats. | Implemented |
| 11 | Network operations: native remotes/refspecs, credentials, clone/fetch, reqwest/rustls with Graviola, progress, and cancellation. | Implemented |
| 12 | Distribution and coverage completion: installed-wheel/sdist checks, supported platforms/ABIs/features, complete API coverage records, examples, and artifact workflows. | Implemented and locally validated; platform CI configured |

## Persistent decisions

- Python distribution: `GixPython`. Import: `gix`. Repository: `GitoxideLabs/GixPython`.
- Preserve existing `gix::Repository` names and semantics. Do not add invented Git convenience methods. A private converter may resolve `str`/`bytes` revspec arguments for native object-ID parameters, without implicit peeling.
- The scope is the native engine's Git-related capabilities. Unsupported native operations are explicit limitations, not invitations to implement a Git subprocess fallback.
- Keep native lazy operations lazy, including incremental Python result conversion and object access. Document any unavoidable eager native computation.
- Use CPython 3.11+; stable ABI for ordinary builds; distinct compatible builds for free-threaded CPython 3.14+, as required by PyO3.
- Thread safety and parallelism are foundational. Detach native work from Python and avoid a single lock serializing all repository operations.
- Default to broad native capability groups and both SHA-1 and SHA-256. All supported builds must avoid compiling C/C++ dependencies; use pure-Rust hashing/compression and reqwest/rustls with Graviola.
- Preserve bytes, typed errors, reference snapshots, and existing engine safety constraints. An expected old reference value must be a literal captured target.
- Progress is pollable through `Progress.snapshot()` and cancellation uses `CancellationToken`; do not add callbacks or process-wide signal ownership.
- License: `MIT OR Apache-2.0`; author and copyright holder: Sebastian Thiel.
- Work locally only. No PyPI interaction, account writes, push, PR, publication, or remote release during the current task. Rust dependency downloads needed for builds are authorized.

After the initial implementation, the user authorized CI setup for this repository,
including pushes to `origin/main` and verification of main/PR workflows. Package
publication and changes to the upstream Gitoxide remote remain outside that scope.

## Upstream integration

Small necessary upstream changes may be prepared in the existing local `gitoxide.pygix` worktree. Its starting base was `f819565c2c`, on branch `pygix`. Inspect its current state before working; this record is not permission to reset or rewrite it.

Do not commit machine-specific dependency paths. Record local overrides and upstream prerequisites explicitly. Publicly reproducible wheels and source distributions require all dependencies to be available independently of that local worktree.

Local Python build selection is implemented through `GIXPYTHON_GITOXIDE_PATH`,
`--gitoxide-path`, and `--packaged` in `etc/build.py`. Cargo 1.97+ provides an ignored
development lockfile; command-local patches build directly from checkout edits
without changing packaged inputs. Local provenance reports checkout HEAD.
Direct Cargo checks and maturin artifacts continue using release dependencies.
Package builds use compatible crates.io releases
and retain macOS wheels plus an sdist. See [development instructions](../DEVELOPMENT.md)
for the switch and migration requirements.

The updated `pygix` branch was reintegrated on 2026-10-08 at
`e4d2b313b840ea0e00453791882dbc6eb7c636f6`. Packaged dependencies use its
public base `1d7bac742f70b72ddda0c7294f4a97566b8db596`, with the same three
prerequisites refreshed under `vendor/`. Bindings propagate the new fallible
path, configuration, reference, shallow-state, and worktree results, and use
`gix::utils` for interruptible reads following the `gix-parallel` split.
The existing release tag and upstream checkout are preserved.

On 2026-10-12, package dependencies moved to public `main` revision
`09f85be4e3fda42f3e8fd19c53a63758fdf6fb3d`, which merged all three prerequisites
in Gitoxide PR #3054. The vendored crates, packaged patches, and vendor-specific
build/archive logic were removed. The latest published `gix 0.89.0` still lacks
these fixes, so migration to crates.io remains pending their next release.
The local checkout selector remains available and packaged provenance uses the
new public revision.

On 2026-10-24, package dependencies moved to crates.io releases `gix 0.90.0`,
`gix-archive 0.39.0`, and `gix-testtools 0.22.0`. All required native APIs are
published, including the fixture symlink preflight. Packaged provenance reports
`dd39fb66e3b1551c9fe849043e39b357a9799ca1` from the published `gix` source.
Every build enables `gix/zlib-sirocco`, including reduced feature selections.
The local checkout selector continues using command-local crates.io patches.

The next packaged build uses `gix 0.91.0`, `gix-archive 0.40.0` and
`gix-testtools 0.23.0` on Rust 1.95 or newer. Python merge results retain the
native index plan so callers can construct the staged merge index independently
of edits to the provisional result tree.

## Acceptance

Native `gix-testtools` fixtures are available through `gix.testtools`, with
Python callbacks preserving `FixtureState` and return-value semantics.
Read-only caches support inspection suites; writable temporary copies keep
mutation tests isolated. See [the fixture guide](testtools.md).

Every completed capability has a Python integration check and accurate type/documentation coverage. Checks use disposable repositories and isolated Git configuration; both hash kinds and relevant feature selections are exercised. Shared-handle concurrency, GIL-disabled execution, partial iterator consumption, interruption, stale-reference rejection, and artifact installation are project requirements.

Gitoxide currently exposes commit merges even where older overview documentation says otherwise. Its push-related configuration does not itself provide native push execution. Verify capabilities against source rather than treating an old feature checklist as authoritative.

## Local review sequence

The substantive steps are committed separately. Follow-up fixes retain their own commits so their regression checks are visible.

| Area | Initial implementation commits |
| --- | --- |
| Project foundation and runtime | `990af09`, `de4bbbd`, `4ce0a11` |
| Repository objects, references, configuration | `169a6d5`, `f5de53d`, `ec44827` |
| Revisions and extended history | `a22cbc3`, `8d5132d` |
| Index, status, and diffs | `79b4aa5`, `f325831` |
| Notes and blame | `b5f9622` |
| Tree/commit and blob merges | `c9e4099`, `bd821ab` |
| Attributes, pathspecs, walking, and filters | `b97c141` |
| Worktrees, archives, and submodules | `9524e67` |
| Network operations and branch relationships | `cd1d7f9`, `e06f76f` |
| Source archives, wheel verification, and CI | `a803637`, `2ef7831` |
| Vendored upstream prerequisites | `63be99c`, `1a6a779` |

`git log --reverse --oneline` includes the focused concurrency, cache, protocol, lifecycle, and documentation follow-ups.

Final lifecycle fixes include clone destination preservation (`33213d4`) and
configuration destruction/retry while an in-memory native owner is active
(`e33597f`). The [validation record](validation.md) lists the passing release
wheel, free-threaded, hash-selection, Rust, and no-C builds, and distinguishes
local evidence from platform checks configured for CI.

## Release preparation

Version `0.3.0` is prepared locally with aligned Cargo metadata and Python
version checks, dated release notes, and updated maintainer instructions.
Local validation is recorded in [validation.md](validation.md). The maintainer
will create the tag; final platform CI, release artifact validation, and
publication remain separate steps in [RELEASING.md](../RELEASING.md).
