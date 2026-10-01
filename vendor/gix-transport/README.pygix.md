# Temporary upstream prerequisite

This directory contains `gix-transport` from local Gitoxide commit
`a13b2ced126803aab080a43dc89b9756fb9bfebd`, based on
`f819565c2c4c56619c4888acef6cf3b8144cbccb`.

The only source change is the reqwest client configuration hook needed to
select a pure Rust TLS provider without a process-wide default. The upstream
commit is local and awaits Sebastian Thiel’s review; nothing has been pushed.
This source copy makes GixPython builds independent of a machine-specific
worktree or an unpublished Git revision. Remove it after the hook lands upstream
and update the pinned Gitoxide revision.

Cargo metadata is normalized to use the pinned Git source for sibling crates;
upstream workspace lints and test-only dependencies are omitted. Upstream tests
were run in the original worktree (77 passed). Original source licenses apply.
