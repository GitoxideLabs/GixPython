# GixPython

Python bindings to [Gitoxide's `gix::Repository`](https://docs.rs/gix/latest/gix/struct.Repository.html), written in Rust with [PyO3](https://pyo3.rs).

The distribution is **GixPython**. Import it as **`gix`**. This is an alpha project; no package has been published as part of its development.

```python
import itertools
import gix

repo = gix.discover(".")
commit = repo.find_commit("HEAD")
print(commit.id, commit.message_raw())

with repo.rev_walk(["HEAD"]).first_parent_only().all() as history:
    for entry in itertools.islice(history, 10):
        print(entry.id, entry.object().message_raw())

with repo.status().into_iter([]) as changes:
    for change in changes:
        print(change.location, change.summary())
```

Native object-ID arguments accept `ObjectId`, object wrappers, or `str`/`bytes` revision specifications. This preserves native object kinds: an annotated tag is not implicitly peeled into a commit. Use a revision such as `v1.0^{commit}` or explicitly peel the returned object. Lazy operations resolve revisions when first consumed. Compare-and-swap reference constraints always retain literal captured targets.

Git contents, paths inside trees, reference names, and messages remain bytes. Filesystem paths are `pathlib.Path` objects. Decode bytes explicitly when displaying text. Methods retain native names and meaning: `Repository.commit()` creates a commit, and `find_commit()` looks one up.

## Capabilities

Bindings cover repositories, objects and tree editing, references and reflogs, configuration transactions, history, index editing, status, diffs, attributes, excludes, pathspecs, filters, directory walking, submodules, notes, blame, blob/tree/commit merges, worktrees, archives, remotes, clone, and fetch. See the [native API coverage record](docs/api-coverage.md) for individual operations, feature gates, and limitations.

The bindings use native Gitoxide operations. The Git executable is used by tests, and is not an implementation fallback. Configured SSH, credential, filter, signing, and merge programs may still run through the native engine.

## Parallelism and lifetimes

Repository handles are safe to share between Python threads. Normal operations use independent native handles and detach from Python while working. Gitoxide parallelism is always enabled. The extension supports GIL-disabled CPython; mutable native builders reject overlapping use rather than depending on the GIL.

Native iterators and output streams remain lazy. Use their context managers or `close()` to release a partially consumed operation promptly. Progress is pollable with `Progress.snapshot()`, and another thread can cancel through `CancellationToken`. Normal Python signals raise `KeyboardInterrupt`. Cancellation is cooperative; some native computations and blocked transport I/O must finish before cleanup can complete. See [runtime semantics](docs/runtime.md).

## Build locally

Use CPython 3.11 or newer, Rust 1.89 or newer, and a system linker:

```sh
python3 etc/build.py
PYTHONPATH=python python3 -m unittest discover -s tests
```

Default `max-pure` builds include both SHA-1 and SHA-256 and broad repository capabilities. Hashing, compression, and HTTPS use Rust implementations; building the dependencies does not require a C or C++ compiler. HTTPS uses reqwest/rustls with Graviola, whose supported CPU targets are checked at runtime.

Select a smaller source build explicitly:

```sh
python3 etc/build.py --no-default-features --features sha256,revision,index
```

`gix.build_features()` reports the installation's capabilities, and `gix.HashKind.all()` reports its supported hash kinds. Feature-gated Python methods and types are absent when disabled. The included type stubs describe the full build.

Ordinary wheel builds select the `abi3` Cargo feature for CPython 3.11+. Free-threaded builds require CPython 3.14+ and a wheel built for that interpreter. [Development instructions](DEVELOPMENT.md) describe artifacts, validation, and the vendored upstream prerequisites. GitHub workflows test supported configurations and produce reviewable artifacts without publishing them.

## More examples

- [Objects, references, and history](examples/repository.py)
- [Pollable progress and cancellation](examples/progress.py)
- [Native blob merges](docs/blob-merges.md)
- [Worktrees, streams, and archives](docs/worktrees.md)
- [Submodules](docs/submodules.md)
- [Clone, fetch, and TLS](docs/network.md)

See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md) for contributions and vulnerability reporting.

## License

Copyright (c) 2026 Sebastian Thiel. Licensed under either [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option, matching Gitoxide.
