# Temporary upstream prerequisite

Source from local Gitoxide commit `97c08543a63252032f9baf7f8de89cd0996db8d9`,
based on `f819565c2c4c56619c4888acef6cf3b8144cbccb`.

The only source change makes reflog writes preserve arbitrary message bytes
using `write_all` instead of BStr text formatting. The upstream regression
was verified failing before the fix and passing after it for both hash kinds.
The commit is local and awaits Sebastian Thiel’s review; nothing was pushed.

Cargo sibling dependencies use the pinned Git source. Workspace lints and
test-only dependencies are omitted. Remove this directory after the fix lands
upstream and update the pinned Gitoxide revision. Original source licenses apply.
