# GixPython

Python bindings to [Gitoxide's `gix::Repository`](https://docs.rs/gix/latest/gix/struct.Repository.html), written in Rust with [PyO3](https://pyo3.rs).

The distribution is **GixPython**. Import it as **`gix`**. This is an alpha project; its API may change before 1.0. The first release targets macOS 11+ on Apple Silicon and Intel, with CPython 3.11+ and separate wheels for free-threaded CPython 3.14. Linux and Windows are not supported release platforms yet.

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

Bindings cover repositories, objects and tree editing, references and reflogs, configuration transactions, history, index editing, status, diffs, attributes, excludes, pathspecs, filters, directory walking, submodules, notes, blame, blob/tree/commit merges, worktrees, archives, remotes, clone, and fetch. See the [native API coverage record](https://github.com/GitoxideLabs/GixPython/blob/main/docs/api-coverage.md) for individual operations, feature gates, and limitations.

The bindings use native Gitoxide operations. The Git executable is used by tests, and is not an implementation fallback. Configured SSH, credential, filter, signing, and merge programs may still run through the native engine.

## Parallelism and lifetimes

Repository handles are safe to share between Python threads. Normal operations use independent native handles and detach from Python while working. Gitoxide parallelism is always enabled. The extension supports GIL-disabled CPython; mutable native builders reject overlapping use rather than depending on the GIL.

Native iterators and output streams remain lazy. Use their context managers or `close()` to release a partially consumed operation promptly. Progress is pollable with `Progress.snapshot()`, and another thread can cancel through `CancellationToken`. Normal Python signals raise `KeyboardInterrupt`. Cancellation is cooperative; some native computations and blocked transport I/O must finish before cleanup can complete. See [runtime semantics](https://github.com/GitoxideLabs/GixPython/blob/main/docs/runtime.md).

## Installation

Once version 0.1.0 is published, install it with:

```sh
python -m pip install --only-binary=:all: GixPython
```

Wheels include the native Rust extension; installation needs no Rust or C compiler. Source builds require Rust 1.89+, a linker, and network access to Rust dependencies.

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

Ordinary wheel builds select the `abi3` Cargo feature for CPython 3.11+. Free-threaded builds require CPython 3.14+ and a wheel built for that interpreter. [Development instructions](https://github.com/GitoxideLabs/GixPython/blob/main/DEVELOPMENT.md) describe artifacts, validation, and the vendored upstream prerequisites. GitHub workflows test supported configurations and build macOS release artifacts. See [RELEASING.md](https://github.com/GitoxideLabs/GixPython/blob/main/RELEASING.md) for the manual publishing procedure.

## More examples

- [Objects, references, and history](https://github.com/GitoxideLabs/GixPython/blob/main/examples/repository.py)
- [Pollable progress and cancellation](https://github.com/GitoxideLabs/GixPython/blob/main/examples/progress.py)
- [Native blob merges](https://github.com/GitoxideLabs/GixPython/blob/main/docs/blob-merges.md)
- [Worktrees, streams, and archives](https://github.com/GitoxideLabs/GixPython/blob/main/docs/worktrees.md)
- [Submodules](https://github.com/GitoxideLabs/GixPython/blob/main/docs/submodules.md)
- [Clone, fetch, and TLS](https://github.com/GitoxideLabs/GixPython/blob/main/docs/network.md)

See [CONTRIBUTING.md](https://github.com/GitoxideLabs/GixPython/blob/main/CONTRIBUTING.md) and [SECURITY.md](https://github.com/GitoxideLabs/GixPython/blob/main/SECURITY.md) for contributions and vulnerability reporting.

## License

Copyright (c) 2026 Sebastian Thiel. Licensed under either [Apache License, Version 2.0](https://github.com/GitoxideLabs/GixPython/blob/main/LICENSE-APACHE) or the [MIT license](https://github.com/GitoxideLabs/GixPython/blob/main/LICENSE-MIT), at your option, matching Gitoxide.
