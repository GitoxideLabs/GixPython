# Runtime and ownership

`Repository` stores a thread-safe native repository. Ordinary operations clone that shared state briefly, create a thread-local `gix::Repository`, and run without the Python interpreter lock. Reading unrelated objects, walking history, or running independent filters can therefore proceed concurrently through one Python repository. Native mutations that replace a repository's configuration or in-memory state reject overlapping mutations.

The extension declares `gil_used = false`. It does not install a signal handler, a global Rust TLS provider, or Python callbacks. Workers own Rust values and never call Python while holding native state locks. Python object conversion and signal checks happen after locks are released.

## Lazy work

Revision walks, reference/reflog iterators, tree entries, status, directory walking, and streamed output start or advance in response to Python demand. A native iterator remains on one owner thread, with a bounded result channel. Closing or dropping a cursor disconnects it and requests cancellation. Explicit `close()` joins native cleanup; garbage collection requests cleanup without waiting indefinitely in a Python destructor.

Some native operations necessarily materialize an outcome: blame, reference maps, tree/commit merges, and `diff_tree_to_tree()` are examples. Their result views may still be lazy. `Tree.changes().for_each_to_obtain_tree()` adapts the native callback traversal into a demand-driven Python iterator. No Python callback executes inside the traversal.

Native builders and borrowed values preserve their ownership boundaries. A remote is borrowed by its connection, a merge resource cache by its prepared merge, and a filter pipeline by its active output stream. An overlapping incompatible call raises `RuntimeError`. Create independent builders for parallel work. Use `with` or `close()` before reusing their parent. Configuration editing, clone persistence, worktree preparation, and stream exhaustion have their native documented commit/discard behavior; context management does not change it.

`with_object_memory()` is an explicit exception to operation-local handles: Gitoxide's in-memory object state must stay on one persistent native handle. Overlapping operations on that view raise `RuntimeError`; normal disk-backed repository handles remain parallel. Objects written through the memory view remain visible to that view and are not persisted to disk.

## Progress and interruption

Pass `progress=gix.Progress()` and `cancel=gix.CancellationToken()` where supported. Another Python thread may poll or cancel them. A progress object serves one active operation at a time. `snapshot()` returns independent task/message records; modifying a snapshot does not modify the operation. States are `idle`, `running`, `succeeded`, `failed`, and `cancelled`. A lazy cursor stays idle until first consumption, succeeds at EOF, and becomes cancelled when closed early. A successfully constructed editable handle ends its construction progress scope when returned.

Cancellation tokens are shared and monotonic. Once cancelled they cannot be reset; create a new token for another operation. Cancelling an unused cursor does not start its native iterator. Cancelling a parked cursor does not advance it again. Cancellation raises `gix.CancelledError`; Python interruption raises `KeyboardInterrupt`.

The caller checks Python signals while detached workers run and waits for native cleanup before returning an interrupted active operation. Native algorithms that accept an interrupt flag stop cooperatively. Native blame and merge operations have no such flag, and transport reads may block until I/O completes. In those cases interruption is recognized while waiting, but cleanup latency is bounded by the native operation or configured transport timeout. No thread is killed asynchronously.

## Errors and snapshots

Invalid Python inputs raise standard Python exceptions. Git failures raise `gix.Error`, preserving native cause text; missing values remain `None` only where the native API reports absence. `gix.Error`, `gix.CancelledError`, and `gix.FeatureUnavailableError` are importable and pickleable.

Objects, references, configuration snapshots, and index snapshots retain the native state they captured. A revspec argument is resolved at the operation's documented execution point. Expected old targets in `PreviousValue` are always literal object IDs or symbolic names, so a concurrent reference update cannot turn a stale comparison into a successful one.
