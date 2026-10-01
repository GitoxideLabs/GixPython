# Native API coverage

GixPython wraps Gitoxide's repository operations and the objects they return.
The source baseline is [`gix` at `f819565c2c4c56619c4888acef6cf3b8144cbccb`](https://github.com/GitoxideLabs/gitoxide/tree/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/src).
Small vendored fixes retain their individual provenance under `vendor/`.
This is broad coverage of the native engine, not a claim to implement every
command available in the Git executable or every generic Rust helper.

`gix.build_features()` reports the compiled capability groups. Type stubs
describe the broad build; a class or method requiring a disabled group is
absent at runtime. Native object-ID arguments also accept `str` or `bytes`
revision specifications when `revision` is enabled. That conversion resolves
an ID without implicitly peeling it to a tree or commit.

## Repository operations

| Native area | Exposed entry points and results | Feature |
| --- | --- | --- |
| Repository lifecycle | Discovery, open, init/init_bare, isolated open options, reload, paths, workdir changes, trust and repository state | Core |
| Object storage | Object/header lookup and optional lookup, existence, raw writes, blobs and streams, empty trees/blobs, hash-aware object IDs | Core |
| Structured objects | Blob bytes; tree entries and path lookup; commit parents, tree, signatures, timestamps and raw messages; tag targets and taggers; decoded owned data | Core |
| Object creation | Native `commit`, `commit_as`, `new_commit`, `new_commit_as`, `tag`, tree editing, upsert/remove/remove_leaf and write | Core |
| References | Reference lookup/iteration, HEAD, namespaces, reference transactions, expected-value constraints, reflogs and branch deletion | Core |
| Configuration | Snapshots, in-memory edits/rollback, physical-file transactions, identities/fallbacks, selected compression/filesystem/stat/command/diff/SSH options | Core; some getters require their capability group |
| Branch configuration | Configured branch names, fetch/push remote selection, remote reference and local tracking reference mapping, reverse tracking lookup; Reference/HEAD shortcuts | Core names; remote-related methods require `network` |
| Revision operations | Revision parsing, lazy walks and ancestry, shallow boundaries, merge bases and octopus merge bases | `revision`; `revparse-regex` enables regex parsing |
| Commit graphs | Native graph inspection, object lookup and metadata, lazy IDs/commits/parent iteration; reusable revision graphs and graph-backed merge-base overloads | Core inspection; `revision` for reusable graphs |
| Description and mailmap | Native describe selection/options/resolution/formatting, commit-graph cache and optional dirty suffix; mailmap resolution and lazy entries | `revision`, `mailmap`; dirty suffix requires `status` |
| Commit signatures | Signature bytes and signed-data extraction; native signing options, signing and verification outcomes | Core extraction; `signing` for configured native helpers |
| Index | Persisted/shared/in-memory indexes, tree conversion, lazy entries, conflict stages, entry edits, sorting, verification and writes | `index` |
| Pathspecs | Native defaults and magic patterns, attribute requirements, matching, lazy pattern and index-entry iteration | `attributes` |
| Attributes and excludes | Combined or separate stacks, index/worktree source selection, overrides, precious patterns, selected/all attribute outcomes and source locations | `attributes` |
| Filters | Reusable native pipelines, to-Git/to-worktree conversion streams, worktree-file object creation and mutable process-driver context | `attributes` |
| Directory walking | Lazy native traversal, pathspec selection, all public native traversal options, entries and final outcome with index/pathspec/exclude state | `dirwalk` |
| Status | Tree/index/worktree changes, untracked/ignored policy, pathspecs, rewrite detection, submodule policy, thread limit, native directory options and index metadata refresh | `status` |
| Diff | Tree comparisons, rename/copy policy, lazy tree changes, reusable resource cache, native line counts/hunks, binary classification and statistics | `blob-diff` |
| Blame and notes | Native blame ranges/options/results; notes lookup, writes/removal and reference selection | `blame`, `notes` |
| Merge | Native tree/commit merges, virtual merge bases, conflict details, tree editors and conflict index entries; blob merge resources/drivers/options | `merge` |
| Worktrees | Linked worktree discovery, proxies, locks, native add/checkout/removal with safety policies | Core inspection; `worktree-mutation` for mutation |
| Export | Native tree streams with incremental entry reads and additional entries; internal, TAR, TAR.GZ and ZIP archives | `worktree-stream`, `worktree-archive`, individual `archive-*` formats |
| Submodules | `.gitmodules` and configuration snapshots, lazy enumeration, raw/validated names, paths/URLs, recorded IDs, activity, repository opening and status | `attributes`; dirty status requires `status` |
| Network | Remote/refspec configuration, URL rewrites, connect/ref-map/fetch, clone builders and checkout, native credentials/helpers | `network`, `http`, `https` |

The checked-in tests exercise these areas in disposable repositories with
isolated Git configuration. They are grouped by capability in `tests/`.
The implementation plan and validation records distinguish implemented APIs
from configurations actually tested on a particular platform.

## Hash and build choices

The default `max-pure` build enables SHA-1 and SHA-256 and the broad capability
groups above. `HashKind.all()` contains only compiled hash kinds;
`HashKind.SHA1` or `.SHA256` is `None` when disabled. Repository creation accepts
`object_hash=...`; opening a repository uses its native object format. Selecting
a hash feature enables an algorithm; it does not convert an existing repository.

`parallel` is always enabled for the native dependency. `max-performance`
selects additional native performance features and does not control thread
safety. The Rust dependency configuration uses Rust hashing/compression and
reqwest/rustls with an explicit Graviola provider. `abi3` selects the ordinary
CPython stable ABI; free-threaded CPython builds require their own ABI.

## Lazy and complete results

Iterators and borrowed native streams retain their Rust owner. Creating a
Python iterator does not convert its entries to a list. `next()` requests an
entry, and `close()` or a context manager releases the owner. Native iterator
implementations may do their own bounded prefetching. Object contents are
loaded when requested by native lookup, not when unrelated IDs are enumerated.

Some native operations already compute a complete result. Examples include
decoded commit/tag/tree fields, configured name sets, reference maps, blame
outcomes, merge conflicts, and `Repository.diff_tree_to_tree()`. These remain
complete results. Blob diff computes its native edit sequence before hunks are
yielded. Attribute matching fills a native outcome before its lazy result
iterator is opened. No stronger streaming guarantee is implied.

An index entry iterator captures a stable native snapshot. Subsequent index
edits use copy-on-write storage and do not change that iterator. Independent
readers use shared read locks; overlapping mutation of the same index raises
`RuntimeError`. Independent repository operations use operation-local native
handles backed by `gix::ThreadSafeRepository`.

Mutable native builders, caches, streams, and attribute/pathspec stacks have
their own synchronization. Overlapping operations on one mutable native owner
may raise `RuntimeError`; create independent owners for parallel operations.
The explicit in-memory object repository uses a shared native mutable state and
can hold its lock for the lifetime of a dependent native owner.

## Attribute and filter usage

```python
index = repo.index_or_empty()
attributes = repo.attributes(index)
selected = attributes.selected_attribute_matches(["text", "eol"])
attributes.at_entry(b"src/file.txt").matching_attributes(selected)
with selected.iter_selected() as matches:
    for match in matches:
        print(match.name, match.state, match.value, match.source)

pipeline, index = repo.filter_pipeline()
with pipeline:
    with pipeline.convert_to_git(b"line\r\n", "src/file.txt", index) as stream:
        git_bytes = stream.read()
```

Attribute sources are `id_mapping`, `id_mapping_then_worktree`, and
`worktree_then_id_mapping`. Ignore sources are `id_mapping` and
`worktree_then_id_mapping_if_not_skipped`. Index-only sources allow matching
without a worktree. A stack configured only for attributes rejects exclude
queries, and an exclude-only stack rejects attribute queries.

Filter inputs are byte buffers; output is a native readable stream. `read(n)`
requests at most `n` bytes; `read()` explicitly requests all remaining bytes.
Close or exhaust a conversion before reusing its pipeline. A dropped reader
releases that loan on the next pipeline operation. The pipeline retains native
process-filter state across conversions. Configured native clean/smudge helpers
are supported. `unknown_encoding="ignore"` or `"fail"` controls the native
policy for unavailable worktree encodings.

## Explicit limits and native behavior

- Rust callback/delegate interfaces are not Python callbacks. Tree change and
  directory traversal use native iterators/adapters. Custom per-entry traversal
  delegates, object-token visitor callbacks and filter callbacks are not exposed.
- Delayed process-filter negotiation is disabled. The native default forbids
  delay; the follow-up delayed-file protocol is not bound. Arbitrary Python
  input-file objects are not accepted by the filter conversion methods.
- Tree-editor cursor shortcuts and the `virtual_merge_base_with_graph()` cache
  reuse overload are not bound. Normal tree editing, native walks, reusable
  merge-base graphs and `virtual_merge_base()` remain available.
- `RevisionWalkPlatform.with_commit_graph()` is not bound; walks can select
  native configured graph use, and reusable revision/describe caches are
  available separately. Attached-ID shortening and `Commit.short_id()` are
  not bound. Commit messages are available as raw bytes and decoded fields,
  without the native parsed `Commit.message()` view.
- `editor()` and `command_context()` expose native selection/context, but
  `editor_command()` does not expose a generic process runner. Rust buffer
  freelists, attach/detach ownership conversions, raw `checkout_options()` and
  `transport_options()` containers, and trait-only storage helpers do not have
  separate Python equivalents. Native checkout and transport operations still
  apply the repository configuration.
- Native pathspec matching suppresses some attribute-loading failures and may
  therefore report no match. Direct attribute-stack lookups preserve these I/O
  errors. `branch_names()` follows native behavior and skips non-UTF-8 names;
  byte-oriented reference and remote APIs preserve their bytes.
- Cancellation is cooperative. Python signals are serviced while native work
  runs detached. Operations without a native interrupt hook, or blocked helper
  or transport I/O, must return before cancellation cleanup can finish. No
  process-wide signal handler is installed.
- Gitoxide exposes push-related configuration but no native push execution at
  this baseline. Full pull/rebase/cherry-pick/stash/bisect and submodule
  add/init/update/deinit porcelain are not supplied by these bindings. There is
  no Git-executable fallback.

See [network details](network.md), [worktree and export details](worktrees.md)
and [submodule details](submodules.md) for native safety constraints and more
specific limits. [History details](history.md) cover graph caches, description,
mailmaps and signature verification.
