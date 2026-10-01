# Changelog

## Unreleased

- Establish GixPython (`import gix`), with MIT OR Apache-2.0 licensing and Sebastian Thiel as author.
- Bind native repository, object, reference, configuration, history, index, status, diff, attribute, filter, submodule, notes, blame, merge, worktree, clone, and fetch operations.
- Support explicit SHA-1/SHA-256 feature choices and broad pure Rust defaults, including reqwest/rustls HTTPS with Graviola.
- Retain lazy iterators and streams, native borrowed lifetimes, byte-oriented Git data, pollable progress, and Python interruption.
- Add shared-handle and GIL-disabled validation, type stubs, examples, contributor/security guidance, and GitHub test/artifact workflows.
- Include three small local upstream fixes with source provenance until they can be reviewed and integrated into Gitoxide.

This is an alpha implementation, not an announcement of publication. Consult [API coverage](docs/api-coverage.md) for limitations.
