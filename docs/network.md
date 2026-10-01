# Native clone and fetch

`prepare_clone()` and `prepare_clone_bare()` create the destination immediately,
like their gix counterparts. Their `PrepareClone` builder wraps
`gix::clone::PrepareFetch`; `fetch_only()` returns a repository and fetch outcome.
With worktree mutation enabled, `fetch_then_checkout()` returns a
`PrepareCheckout`, whose `main_worktree()` performs the native checkout.
Dropping an unfinished clone/checkout keeps gix's automatic cleanup behavior.
`persist()` explicitly keeps an unfinished repository. Successful consuming
operations make subsequent use raise `RuntimeError`, preventing native panics.

```python
import gix

progress = gix.Progress()
cancel = gix.CancellationToken()
clone = gix.prepare_clone("https://example.com/project.git", "project")
checkout, fetched = clone.fetch_then_checkout(progress=progress, cancel=cancel)
repo, checked_out = checkout.main_worktree(progress=progress, cancel=cancel)
```

`Repository.find_remote()`, `find_fetch_remote()`, and `remote_at()` produce
native remotes. Configure their refspecs and tag policy, call
`connect(Direction.Fetch)`, then `prepare_fetch().receive()` to fetch. `ref_map()`
performs the native handshake and reference mapping without fetching objects.
The reference map is a complete native result, not a lazy iterator.

Builders retain their native state in a Python-free owner thread. A remote is
busy while a child connection borrows it; independent remotes from a shared
repository can run in parallel. Builder methods modify their Python handle and
return it for chaining. Git credentials, URL rewrites, allowed protocols, and
configured helper programs continue to use gix's native implementation.
Python callbacks are not accepted.

Pass `Progress` and `CancellationToken` to long operations. Another thread can
poll `progress.snapshot()` or call `cancel.cancel()`. Cancellation is cooperative:
blocking transport I/O must return before native cancellation checks can finish.
The calling thread continues to service Python signals. No process-wide signal
handler or Rust TLS-provider default is installed.

HTTPS uses reqwest/rustls with an explicit Graviola provider. CPU requirements
are checked before the provider is used. Trust comes from the platform verifier
by default, or from `http.sslCAInfo` when set. Git's SSL verification setting,
TLS version/range, HTTP version, user agent, proxy, connection timeout, extra
headers, and redirect protections are retained. Unsupported low-speed thresholds
and proxy credential-helper configurations return errors rather than being
silently ignored. Custom TLS client identities and Python credential callbacks
are not exposed.

The client-builder prerequisite is Gitoxide commit
`a13b2ced126803aab080a43dc89b9756fb9bfebd`, prepared locally in the sibling checkout.
The reproducible build uses the source-preserving vendored transport patch;
see [its provenance and removal conditions](../vendor/gix-transport/README.pygix.md).
No unpublished remote revision or machine-specific dependency path is required.
