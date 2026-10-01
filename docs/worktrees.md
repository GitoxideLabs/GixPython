# Worktrees, streams, and archives

`repo.worktree()` returns the current native worktree, or `None` for a bare
repository. `worktrees()` returns native linked-worktree proxies;
`worktrees_including_main()` lazily opens repositories and accepts `Progress`
and `CancellationToken`. Proxies preserve their selected native ID and expose
locking, pruning, paths, and repository access.

With `attributes`, the native `worktree.attributes()`, `attributes_only()`, and
`pathspec(patterns)` shortcuts use the worktree's index and native defaults.
The resulting attribute stack and pathspec retain their own state.

With `worktree-mutation`, checkout and removal use Gitoxide's native safety
checks:

```python
import gix

repo = gix.open("project")
linked, outcome = repo.add_worktree(
    "../project-inspect", gix.WorktreeHead.Detached(repo.head_id())
)
print(outcome.files_updated, outcome.bytes_written)

with repo.prepare_remove_worktree("../project-inspect") as target:
    target.options(gix.WorktreeRemoveOptions(thread_limit=2))
    target.remove()  # Refuses dirty or locked worktrees by default.
```

`WorktreeHead.Attached(full_name)` checks native branch occupancy. Deleting a
local branch also respects worktree occupancy. Removal defaults to
`WorktreeRemoveForce.Never`; `DiscardChanges` permits removing local changes,
and `OverrideLock` also permits removing a locked worktree. A prepared target
retains native selection until removed or closed. Removal consumes the target,
including when the native operation fails. Preparation does not bypass checks
performed at removal time.

`add_worktree()` accepts progress and cancellation. Removal accepts progress;
the native removal API has no interruption flag. `CheckoutOutcome` reports
updated file/byte counts, collisions, errors, and delayed paths.

With `worktree-stream`, request entries incrementally:

```python
stream, index = repo.worktree_stream(repo.head_tree_id())
with stream:
    while (entry := stream.next_entry()) is not None:
        print(entry.relative_path(), entry.mode, entry.bytes_remaining())
        while chunk := entry.read(64 * 1024):
            consume(chunk)
```

The native stream and borrowed entry stay on one Rust worker. Entry metadata
does not preload contents. `read(size)` limits returned bytes; `read()` explicitly
requests all remaining content. Advancing drains unread content in bounded
chunks, then invalidates the previous entry. Using an invalidated entry raises
`ValueError`. Closing or dropping the stream releases the worker without
reading all remaining content. Entry objects do not keep a dropped stream alive.
The stream supports progress, cancellation, and context management. Reading to
EOF marks progress succeeded; early close marks it cancelled.

`add_entry(AdditionalEntry(...))` and `add_entry_from_path(...)` append native
extra entries before traversal starts. Sources are `StreamSource.Null`,
`StreamSource.Memory(bytes)`, and `StreamSource.Path(path)`.

With `worktree-archive`, `repo.worktree_archive(stream, output_path, options)`
consumes a stream and writes a native archive to a seekable file. Configure it
with `ArchiveOptions(format, compression_level=..., tree_prefix=...,
modification_time=...)`. Formats are `internal`, `tar`, `tar.gz`, and `zip`;
individual archive features select compiled formats. Gitoxide's ZIP writer
requires UTF-8 entry paths. TAR and internal stream formats preserve raw path
bytes. Errors may leave a partial output file, matching the native writer.
