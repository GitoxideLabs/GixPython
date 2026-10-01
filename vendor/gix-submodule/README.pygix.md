# Temporary upstream prerequisite

Source from local Gitoxide commit `fe215611d5960db7b4c534a6dba46b8b338fcf15`,
based on `f819565c2c4c56619c4888acef6cf3b8144cbccb`.

The source change looks up submodule fields and activity by literal byte names,
without formatting names through UTF-8. The regression failed before the fix;
all 26 native submodule tests passed afterward with both hash kinds enabled.
The commit is local and awaits Sebastian Thiel’s review; nothing was pushed.

Cargo sibling dependencies use the pinned Git source. Workspace lints and
test-only dependencies are omitted. Remove this directory after the fix lands
upstream and update the pinned Gitoxide revision. Original source licenses apply.
