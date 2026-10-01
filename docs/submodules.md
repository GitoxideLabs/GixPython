# Submodules

Submodule support is included with the `attributes` feature. It exposes native
Gitoxide configuration, discovery, activity, repository access, and status.
Gitoxide does not currently provide submodule add/init/update/deinit porcelain
operations through this API; GixPython does not implement substitutes.

```python
import gix

repo = gix.open("project")
modules = repo.submodules()
if modules is not None:
    with modules:
        for submodule in modules:
            print(submodule.name(), submodule.path(), submodule.url())
            nested = submodule.open()  # None when no repository is initialized
            print(submodule.state().repository_exists)
```

`open_modules_file()` reads only the worktree file. `modules()` also tries the
index and HEAD tree when the worktree file is absent. `submodules()` uses the
same native lookup. A missing configuration returns `None`; an empty
configuration produces an empty iterator. Invalid fields raise `gix.Error` when
queried. Repository corruption is an error, rather than an absent submodule.

Names, paths within Git, URLs, and named branch/update values retain their
bytes. Filesystem paths use Python's filesystem conversion. A submodule name is
unvalidated until `validated_name()` or an operation requiring it is called.
`index_id()` and `head_id()` refer to the superproject's recorded gitlink;
`open().head_id()` refers to the checked-out submodule repository.

The submodule iterator and its yielded objects keep the native configuration
snapshot and shared lazy state on one Rust worker. Closing or dropping the
iterator stops traversal while previously yielded objects remain usable.
Methods can run on different Python threads. Overlapping calls on this shared
state raise `RuntimeError`; create another iterator for independent concurrent
operations. Native work runs detached from Python. The last retained object
releases its worker.

`ModulesFile.from_bytes(data, path=None, config=None)` parses standalone
`.gitmodules` data. `config` accepts a `ConfigFile`, such as
`repo.config_snapshot().plumbing()`. `append_submodule_overrides(config)` applies
the native override fields to an existing modules file and returns itself.
`config()` returns an independently mutable configuration copy. `names()` is a
lazy iterator over a captured snapshot.

Field access preserves native distinctions: unset values return `None`;
`SubmoduleIgnore.None_` and `SubmoduleUpdate.None_` are actual configured values.
`SubmoduleBranch.CurrentInSuperproject` represents `branch = .`.
`SubmoduleUpdate.Command(...)` contains bytes but does not execute them.
Native validation rejects custom update commands distributed in `.gitmodules`;
commands supplied by a repository configuration override remain data.

With the `status` feature, `submodule.status(ignore, check_dirty)` returns the
native `SubmoduleStatus`, including recorded and checked-out IDs, state, changes,
and `is_dirty()`. An unknown dirty state returns `None`. Native status computes
its result before returning; it is not a lazy change stream.

The small temporary native raw-name fix is documented in
[`vendor/gix-submodule/README.pygix.md`](../vendor/gix-submodule/README.pygix.md).
