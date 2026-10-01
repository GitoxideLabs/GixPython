# Native history metadata

`Repository.commit_graph()` opens Git's optional commit-graph cache; the
`commit_graph_if_enabled()` variant returns `None` when it is disabled or absent.
The graph exposes native commit IDs, generation numbers, timestamps, tree IDs,
and parent positions. Positions are checked before native lookup. Its
`iter_ids()`, `iter_commits()`, and `CommitGraphCommit.iter_parents()` iterators
are lazy and support `close()` and context-manager cleanup.

With `revision` enabled, `Repository.revision_graph(cache=None)` retains a native
revision graph for reuse by `merge_base_with_graph()`,
`merge_bases_many_with_graph()`, and `merge_base_octopus_with_graph()`. A graph
owns its repository handle and serializes its own cache operations; independent
graphs can run in parallel. `clear()` drops cached entries and `close()` releases
the graph. `Commit.describe()` uses native tag selection, first-parent traversal,
candidate limits, optional commit-graph cache, and object-ID fallback.

```python
import gix

repo = gix.discover(".")
graph = repo.revision_graph(repo.commit_graph_if_enabled())
try:
    base = repo.merge_base_with_graph("HEAD", "main", graph)
finally:
    graph.close()

description = repo.find_commit(repo.head_id()).describe().names(gix.SelectRef.AllTags)
print(description.format())
```

Describe configuration methods return configured copies. As in gix,
`DescribePlatform.format()` enables object-ID fallback on that platform.
`DescribeResolution.format_with_dirty_suffix()` additionally requires `status`.
Names are available as bytes; `str(DescribeFormat)` uses native display formatting.

With `mailmap` enabled, `Repository.open_mailmap()` reads Git's configured
mailmap sources. `Mailmap.resolve()` and `try_resolve()` preserve signature times
and use native byte-oriented matching. `open_mailmap_into()` merges into an
existing snapshot. Existing `iter()` iterators retain their original snapshot;
the explicit `entries()` method returns the complete native list. The native
`open_mailmap()` method tolerates source-loading errors; use
`open_mailmap_into()` to receive those errors, including after a partial merge.

`Commit.signature()` returns signature bytes and a `SignedData` view without
materializing the signed payload until `to_bstring()` is requested. With
`signing` enabled, `Commit.signed()` writes a newly signed commit object while
leaving references unchanged. `verify_signature()` returns the native status,
trust level, signer, fingerprints, and verifier output; unsigned commits return
`None`. `Repository.commit_signing_options()` exposes the effective format,
program, and key; its `if_enabled` variant respects `commit.gpgSign`.
Signing and verification run native configured helper programs, with no Python
callback. SSH signing tests generate local disposable keys and require no
external service.
