# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### New Features

 - Select a local Gitoxide checkout for Python development builds with
   `GIXPYTHON_GITOXIDE_PATH` or `--gitoxide-path`, using a separate development
   lockfile. `--packaged` forces release dependencies. Local builds report checkout HEAD.

## 0.1.0 (2026-10-05)

### New Features

 - Initial GixPython release, imported as `gix`, for macOS 11+ on Apple Silicon and Intel.
   Ordinary CPython 3.11+ uses stable-ABI wheels; free-threaded CPython 3.14 uses separate wheels.
 - Bind native repository, object, reference, configuration, history, index, status,
   diff, attribute, filter, submodule, notes, blame, merge, worktree, clone, and fetch operations.
   Preserve native names and semantics, with revision specifications accepted for object-ID arguments.
 - Support both SHA-1 and SHA-256 with selectable capability groups and pure Rust defaults,
   including reqwest/rustls HTTPS with Graviola. Source builds require Rust and a linker.
 - Keep repository operations parallel and safe to share across Python threads, including with
   the GIL disabled. Retain lazy iterators, byte-oriented Git data, pollable progress, and interruption.
 - Include Python type information and examples. See [API coverage](docs/api-coverage.md)
   for available operations and alpha-version limitations.

### Other

 - Use MIT OR Apache-2.0 licensing, with Sebastian Thiel as author and copyright holder.
 - Include three small Gitoxide source patches with their licenses and provenance until
   the corresponding upstream fixes are integrated.

This entry describes the prepared release; publication is a separate maintainer action.
