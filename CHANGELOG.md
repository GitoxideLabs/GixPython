# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Breaking Changes

 - Upgrade to `gix 0.91.0`, `gix-archive 0.40.0`, and `gix-testtools 0.23.0`.
   Source builds require Rust 1.95.
 - Use `TreeMergeOutcome.index_file()` for the full merged index, including
   unresolved stages. The native index plan replaces per-conflict index edits.
 - Rewrite details provide native similarity scores. Compute patch line counts
   separately through `BlobDiff.line_counts()`.
 - Missing resource and stream IDs use `None`. `add_entry_from_path()` takes
   a root and path. `Submodule.work_dir()` returns `None` without a worktree.

## 0.3.0 (2026-10-24)

### Other

 - Use crates.io releases `gix 0.90.0`, `gix-archive 0.39.0`, and
   `gix-testtools 0.22.0` in place of the Git revision pin.
 - Enable `gix/zlib-sirocco` in all builds, including reduced feature
   configurations, for pure Rust decompression.

## 0.2.0 (2026-10-14)

### New Features

 - Expose native cached read-only and writable fixtures through `gix.testtools`,
   with Python closures for fixture creation and post-processing.

### Bug Fixes

 - Propagate native errors from fallible path, configuration, reference,
   shallow-state, and worktree queries after updating Gitoxide.

### Other

 - Pin Gitoxide to public `main` revision
   `09f85be4e3fda42f3e8fd19c53a63758fdf6fb3d`, containing the upstream prerequisites
   for per-client TLS configuration, raw reflog messages, and literal submodule
   names. Remove the vendored crates and packaged patches. `gix 0.89.0` does not
   yet include these fixes.
 - Group developer commands in the `justfile` and add `just run-release` for
   isolated exploration of the latest or a specified published version, with
   IPython completion for interactive sessions.
 - Run platform-independent release validation and source archive creation on
   Ubuntu, while retaining macOS builds and tests for all four release wheels.

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
