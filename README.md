# GixPython

Python bindings to the [Gitoxide `gix` crate](https://github.com/GitoxideLabs/gitoxide), implemented with [PyO3](https://pyo3.rs).

The distribution is named **GixPython**; Python code imports **`gix`**. This repository is `GitoxideLabs/pygix`.

## Development status

This project is under active construction. The binding surface and packaging are being added in independently reviewable steps. [The implementation plan](docs/implementation-plan.md) records the intended coverage; an enabled Cargo feature alone does not mean its Python bindings are complete.

The API follows native `gix` names and semantics. Object lookup, revision parsing, commit creation, and reference transactions remain distinct operations. Iterators remain lazy, and byte-oriented Git data remains available without lossy conversion.

## Runtime and build goals

- CPython 3.11+, with stable-ABI wheels for ordinary CPython and separate wheels for free-threaded CPython 3.13/3.14.
- Thread-safe repository handles, native parallelism, and Python detachment during Git operations.
- Both SHA-1 and SHA-256 by default, with explicit source-build feature choices.
- Pure-Rust hashing, compression, and reqwest/rustls networking with Graviola, without compiling C or C++ dependencies.
- Broad coverage of native repository, object, reference, index, worktree, history, diff, merge, and transport capabilities.

Native Gitoxide capabilities set the upper bound of the bindings. Features missing from the engine are documented rather than emulated through the Git executable. Gitoxide may run configured external programs for SSH, credentials, filters, or signing.

## Working from source

See [DEVELOPMENT.md](DEVELOPMENT.md) for build prerequisites, local upstream integration, validation, and artifact generation. The current work is local only; no package publication or remote release has been performed.

Contributions follow [CONTRIBUTING.md](CONTRIBUTING.md). Please report vulnerabilities using [SECURITY.md](SECURITY.md).

## License

Copyright (c) 2026 Sebastian Thiel.

Licensed under either [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option, matching Gitoxide.
