# Blob merge resources

With `merge`, `repo.merge_resource_cache()` creates Gitoxide's native three-way
blob merge platform. It uses repository configuration, index attributes, filter
settings, and merge drivers. Resource kinds are `current`, `ancestor`, and
`other`; modes are native tree-entry modes for blobs or executable blobs.

```python
cache = repo.merge_resource_cache()
cache.set_resource(base_blob, 0o100644, b"file", "ancestor")
cache.set_resource(our_blob, 0o100644, b"file", "current")
cache.set_resource(their_blob, 0o100644, b"file", "other")

with cache.prepare_merge(repo.blob_merge_options()) as prepared:
    buffer, pick, resolution = prepared.merge()
    merged_id = prepared.id_by_pick(pick, buffer)
```

`merge()` and `builtin_merge(driver)` return the native output buffer plus
`pick` and `resolution`. Picks are `Buffer`, `Ours`, `Theirs`, or `Ancestor`.
Resolutions are `Complete`, `CompleteWithAutoResolvedConflict`, or `Conflict`.
A binary merge can pick an existing resource and leave the output buffer empty.
`buffer_by_pick(pick)` returns that resource's bytes, or `None` for `Buffer`;
it raises `ValueError` if the selected resource was too large to load.
`id_by_pick(pick, buffer)` returns an existing ID or writes the selected content
through the repository's native blob writer. It can return `None` for absent
resources or oversized worktree content without a known ID. Neither method
updates the index or worktree.

`prepare_merge()` retains the actual native borrowed preparation on one worker.
Its `current`, `ancestor`, and `other` expose resource metadata and request bytes
only through `data` or `as_slice()`. `Missing` resources have `data = None` and
`as_slice() = b""`; `TooLarge` resources return `None` for both. `Buffer`
resources return their converted bytes. Preparing a merge does not copy all
three native buffers into Python.

The native cache is borrowed until the preparation is closed or its last
resource object is dropped. Attempts to mutate or prepare that cache while it
is borrowed raise `RuntimeError`. Closing a preparation invalidates later data
access through retained resources. Resource metadata already returned remains
available. Methods detach from Python; overlapping calls on the same prepared
state raise `RuntimeError`. Use independent caches for parallel merges.

`BlobMergeOptions` exposes virtual-ancestor behavior, binary resolution,
diff algorithm, text conflict resolution, conflict style, and marker size.
Styles are `merge`, `diff3`, and `zdiff3`; text resolution is `keep`, `ours`,
`theirs`, or `union`; binary resolution is `None`, `ancestor`, `ours`, or
`theirs`. Options from `repo.blob_merge_options()` include Git configuration.
Preparation can adjust these options from attributes or recursive driver rules.
Inspect or replace `prepared.options` after preparation, and use `MergeLabels`
for byte-preserving conflict labels.

`prepared.driver` is a built-in driver name (`text`, `binary`, or `union`) or an
index into `cache.drivers()`. `configured_driver()` returns the selected driver
descriptor or built-in name. In the pinned Gitoxide revision, a driver's
`display_name` is its key name; the optional `merge.<name>.name` description is
not loaded. Native `merge()` executes configured external merge
drivers and preserves their errors. `builtin_merge()` explicitly selects a
built-in driver. Standalone external-command preparation/process management is
not exposed; command execution remains part of the native merge operation.

Optional `current_root`, `other_root`, and `common_ancestor_root` select worktree
sources. A null object ID denotes absent content unless a worktree source is
available. Native filters and normalization apply when resources are loaded.
The cache exposes `default_driver`, `filter_mode` (`to_git` or `renormalize`),
and `large_file_threshold_bytes`; set these before `set_resource()` so they
apply to conversion.

Cache construction and preparation accept progress and cancellation. Their
scope ends when the handle is returned. Native merge has no interruption flag;
Python signal handling is preserved while waiting for the worker, and a running
native operation completes before its state is released.
