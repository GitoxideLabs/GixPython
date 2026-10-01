//! Python-free workers, cooperative cancellation, and pollable native progress.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use gix::progress::prodash::{messages::MessageLevel, progress::State, tree};
use pyo3::{
    exceptions::PyRuntimeError,
    prelude::*,
    types::{PyBytes, PyDict, PyTuple},
};

pyo3::create_exception!(gix, CancelledError, crate::error::Error);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Cooperative cancellation shared by any number of Python threads.
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

#[pymethods]
impl CancellationToken {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. A cancelled token cannot be reset.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    #[getter]
    pub fn cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

struct ProgressState {
    state: &'static str,
    tree: Arc<tree::Root>,
}

/// Thread-safe progress snapshots for one active operation at a time.
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Progress {
    inner: Arc<Mutex<ProgressState>>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProgressState {
                state: "idle",
                tree: tree::Root::new(),
            })),
        }
    }
}

#[pymethods]
impl Progress {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[getter]
    fn state(&self) -> &'static str {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).state
    }

    /// Return independent copies of the operation state, tasks, and recent messages.
    /// Task totals are None when upstream cannot estimate them.
    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let (state, tree) = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            (inner.state, inner.tree.clone())
        };
        let mut tasks = Vec::new();
        let mut messages = Vec::new();
        tree.sorted_snapshot(&mut tasks);
        tree.copy_messages(&mut messages);

        // All native locks are released before creating Python objects.
        let mut task_records = Vec::with_capacity(tasks.len());
        for (key, task) in tasks {
            let record = PyDict::new(py);
            record.set_item("path", PyTuple::new(py, (1..=key.level()).map(|i| key[i]))?)?;
            record.set_item("id", PyBytes::new(py, &task.id))?;
            record.set_item("name", task.name)?;
            let (current, total, unit, status) = match task.progress {
                None => (None, None, None, "running"),
                Some(value) => {
                    let current = value.step.load(Ordering::Relaxed);
                    let unit = value.unit.map(|unit| {
                        let mut label = String::new();
                        let _ = unit.as_display_value().display_unit(&mut label, current);
                        label
                    });
                    let status = match value.state {
                        State::Running => "running",
                        State::Blocked(..) => "blocked",
                        State::Halted(..) => "halted",
                    };
                    (Some(current), value.done_at, unit, status)
                }
            };
            record.set_item("current", current)?;
            record.set_item("total", total)?;
            record.set_item("unit", unit)?;
            record.set_item("state", status)?;
            task_records.push(record);
        }
        let message_records = messages.into_iter().map(|message| {
            let level = match message.level {
                MessageLevel::Info => "info",
                MessageLevel::Failure => "failure",
                MessageLevel::Success => "success",
            };
            (level, message.origin, message.message)
        });
        let snapshot = PyDict::new(py);
        snapshot.set_item("state", state)?;
        snapshot.set_item("tasks", PyTuple::new(py, task_records)?)?;
        snapshot.set_item("messages", PyTuple::new(py, message_records)?)?;
        Ok(snapshot)
    }
}

struct ProgressLease {
    owner: Option<Progress>,
    tree: Arc<tree::Root>,
}

impl ProgressLease {
    fn begin(progress: Option<&Progress>, name: &'static str) -> PyResult<(Self, tree::Item)> {
        let tree = tree::Root::new();
        if let Some(progress) = progress {
            let mut inner = progress.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.state == "running" {
                return Err(PyRuntimeError::new_err("progress is already in use"));
            }
            inner.tree = tree.clone();
            inner.state = "running";
        }
        let item = tree.add_child(name);
        Ok((
            Self {
                owner: progress.cloned(),
                tree,
            },
            item,
        ))
    }

    fn finish(&self, state: &'static str) {
        if let Some(owner) = &self.owner {
            let mut inner = owner.inner.lock().unwrap_or_else(|e| e.into_inner());
            if Arc::ptr_eq(&inner.tree, &self.tree) {
                inner.state = state;
            }
        }
    }
}

/// Native work receives only Rust data. The interrupt flag belongs to this operation,
/// not to the user's token (some upstream iterator destructors modify their flag).
pub struct OperationContext {
    pub progress: tree::Item,
    pub interrupt: Arc<AtomicBool>,
}

fn check_interrupt(py: Python<'_>, cancel: Option<&CancellationToken>, interrupt: &AtomicBool) -> PyResult<()> {
    if let Err(error) = py.check_signals() {
        interrupt.store(true, Ordering::Release);
        return Err(error);
    }
    if interrupt.load(Ordering::Acquire) || cancel.is_some_and(CancellationToken::cancelled) {
        interrupt.store(true, Ordering::Release);
        return Err(CancelledError::new_err("operation cancelled"));
    }
    Ok(())
}

fn receive<T: Send>(
    py: Python<'_>,
    receiver: &mut Receiver<T>,
    cancel: Option<&CancellationToken>,
    interrupt: &AtomicBool,
) -> PyResult<Option<T>> {
    loop {
        check_interrupt(py, cancel, interrupt)?;
        let receiver = &mut *receiver;
        match py.detach(move || receiver.recv_timeout(POLL_INTERVAL)) {
            Ok(value) => {
                check_interrupt(py, cancel, interrupt)?;
                return Ok(Some(value));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}

fn join_worker(
    py: Python<'_>,
    handle: JoinHandle<()>,
    interrupt: &AtomicBool,
    mut error: Option<PyErr>,
) -> PyResult<()> {
    while !handle.is_finished() {
        py.detach(|| thread::sleep(POLL_INTERVAL));
        if let Err(signal) = py.check_signals() {
            interrupt.store(true, Ordering::Release);
            if error.is_none() {
                error = Some(signal);
            }
        }
    }
    if py.detach(move || handle.join()).is_err() && error.is_none() {
        error = Some(PyRuntimeError::new_err("native worker panicked"));
    }
    error.map_or(Ok(()), Err)
}

/// Run potentially long native work while the calling thread services Python signals.
/// Native errors stay native so callers can map them to the appropriate Python exception.
pub fn run<T, E, F>(
    py: Python<'_>,
    name: &'static str,
    progress: Option<&Progress>,
    cancel: Option<&CancellationToken>,
    work: F,
) -> PyResult<Result<T, E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce(OperationContext) -> Result<T, E> + Send + 'static,
{
    let interrupt = Arc::new(AtomicBool::new(false));
    check_interrupt(py, cancel, &interrupt)?;
    let (lease, progress) = ProgressLease::begin(progress, name)?;
    let context = OperationContext {
        progress,
        interrupt: interrupt.clone(),
    };
    let (sender, mut receiver) = mpsc::sync_channel(1);
    let handle = match thread::Builder::new().name(format!("pygix-{name}")).spawn(move || {
        let _ = sender.send(work(context));
    }) {
        Ok(handle) => handle,
        Err(error) => {
            lease.finish("failed");
            return Err(PyRuntimeError::new_err(error.to_string()));
        }
    };
    let result = receive(py, &mut receiver, cancel, &interrupt);
    drop(receiver);
    let (value, error) = match result {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error)),
    };
    if let Err(error) = join_worker(py, handle, &interrupt, error) {
        lease.finish(if interrupt.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "failed"
        });
        return Err(error);
    }
    match value {
        Some(value) => {
            lease.finish(if value.is_ok() { "succeeded" } else { "failed" });
            Ok(value)
        }
        None => {
            lease.finish("failed");
            Err(PyRuntimeError::new_err("native worker stopped without a result"))
        }
    }
}

enum IterEvent<T, E> {
    Item(T),
    Done(Result<(), E>),
}

/// An iterator owner can borrow its local repository while serving demand on its stack.
pub struct IterProducer<T, E> {
    requests: Receiver<()>,
    results: SyncSender<IterEvent<T, E>>,
    interrupt: Arc<AtomicBool>,
}

impl<T: Send, E: Send> IterProducer<T, E> {
    pub fn requested(&self) -> bool {
        self.requests.recv().is_ok() && !self.interrupt.load(Ordering::Acquire)
    }
    pub fn send(&self, item: T) -> bool {
        self.results.send(IterEvent::Item(item)).is_ok()
    }
    pub fn serve(self, mut iter: impl Iterator<Item = Result<T, E>>) -> Result<(), E> {
        while self.requested() {
            match iter.next() {
                Some(Ok(item)) => {
                    if !self.send(item) {
                        break;
                    }
                }
                Some(Err(error)) => return Err(error),
                None => break,
            }
        }
        Ok(())
    }
}

type IterFactory<T, E> = Box<dyn FnOnce(OperationContext, IterProducer<T, E>) -> Result<(), E> + Send>;

struct IterWorker<T, E> {
    requests: SyncSender<()>,
    results: Receiver<IterEvent<T, E>>,
    handle: JoinHandle<()>,
    lease: ProgressLease,
}

enum IterState<T, E> {
    Pending(IterFactory<T, E>),
    Running(IterWorker<T, E>),
    Busy,
    Closed,
}

/// A demand-driven cursor. A single owner worker starts on first use, never once per item.
/// Public Python wrappers provide __next__, close, and context management around this type.
pub struct OwnedIter<T, E> {
    state: Mutex<IterState<T, E>>,
    name: &'static str,
    progress: Option<Progress>,
    cancel: Option<CancellationToken>,
    interrupt: Arc<AtomicBool>,
}

impl<T: Send + 'static, E: Send + 'static> OwnedIter<T, E> {
    pub fn new(
        name: &'static str,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
        factory: impl FnOnce(OperationContext, IterProducer<T, E>) -> Result<(), E> + Send + 'static,
    ) -> Self {
        Self {
            state: Mutex::new(IterState::Pending(Box::new(factory))),
            name,
            progress: progress.cloned(),
            cancel: cancel.cloned(),
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start(&self, factory: IterFactory<T, E>) -> PyResult<IterWorker<T, E>> {
        let (lease, progress) = ProgressLease::begin(self.progress.as_ref(), self.name)?;
        let context = OperationContext {
            progress,
            interrupt: self.interrupt.clone(),
        };
        let (requests, request_receiver) = mpsc::sync_channel(1);
        let (result_sender, results) = mpsc::sync_channel(1);
        let producer = IterProducer {
            requests: request_receiver,
            results: result_sender.clone(),
            interrupt: self.interrupt.clone(),
        };
        match thread::Builder::new()
            .name(format!("pygix-{}", self.name))
            .spawn(move || {
                let result = factory(context, producer);
                let _ = result_sender.send(IterEvent::Done(result));
            }) {
            Ok(handle) => Ok(IterWorker {
                requests,
                results,
                handle,
                lease,
            }),
            Err(error) => {
                lease.finish("failed");
                Err(PyRuntimeError::new_err(error.to_string()))
            }
        }
    }

    pub fn next(&self, py: Python<'_>) -> PyResult<Option<Result<T, E>>> {
        let state = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(*state, IterState::Busy) {
                return Err(PyRuntimeError::new_err("iterator is already executing"));
            }
            std::mem::replace(&mut *state, IterState::Busy)
        };
        let result = self.next_inner(py, state);
        if result.is_err() {
            *self.state.lock().unwrap_or_else(|e| e.into_inner()) = IterState::Closed;
        }
        result
    }

    fn next_inner(&self, py: Python<'_>, state: IterState<T, E>) -> PyResult<Option<Result<T, E>>> {
        let mut worker = match state {
            IterState::Closed => {
                *self.state.lock().unwrap_or_else(|e| e.into_inner()) = IterState::Closed;
                return Ok(None);
            }
            IterState::Pending(factory) => {
                check_interrupt(py, self.cancel.as_ref(), &self.interrupt)?;
                self.start(factory)?
            }
            IterState::Running(worker) => worker,
            IterState::Busy => unreachable!("busy state is checked before taking ownership"),
        };
        // One outstanding request is guaranteed by the Busy state. The worker may have
        // already reported an initialization error and disconnected the request channel.
        let result = match check_interrupt(py, self.cancel.as_ref(), &self.interrupt) {
            Ok(()) => {
                let _ = worker.requests.try_send(());
                receive(py, &mut worker.results, self.cancel.as_ref(), &self.interrupt)
            }
            Err(error) => Err(error),
        };
        if let Ok(Some(IterEvent::Item(item))) = result {
            *self.state.lock().unwrap_or_else(|e| e.into_inner()) = IterState::Running(worker);
            return Ok(Some(Ok(item)));
        }
        let IterWorker {
            requests,
            results,
            handle,
            lease,
        } = worker;
        drop((requests, results));
        let (outcome, error) = match result {
            Ok(Some(IterEvent::Done(outcome))) => (Some(outcome), None),
            Err(error) => (None, Some(error)),
            _ => (None, None),
        };
        let joined = join_worker(py, handle, &self.interrupt, error);
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = IterState::Closed;
        if let Err(error) = joined {
            lease.finish(if self.interrupt.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "failed"
            });
            return Err(error);
        }
        match outcome {
            Some(Ok(())) => {
                lease.finish("succeeded");
                Ok(None)
            }
            Some(Err(error)) => {
                lease.finish("failed");
                Ok(Some(Err(error)))
            }
            None => {
                lease.finish("failed");
                Err(PyRuntimeError::new_err("native iterator stopped without a result"))
            }
        }
    }

    /// A returned native handle outlives the operation that constructed it. End that
    /// operation's progress/cancellation scope after its first successful response.
    pub fn finish_initialization(&mut self) {
        if let IterState::Running(worker) = self.state.get_mut().unwrap_or_else(|e| e.into_inner()) {
            worker.lease.finish("succeeded");
            worker.lease.owner = None;
        }
        self.progress = None;
        self.cancel = None;
    }

    pub fn close(&self, py: Python<'_>) -> PyResult<()> {
        let state = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(*state, IterState::Busy) {
                return Err(PyRuntimeError::new_err("iterator is already executing"));
            }
            std::mem::replace(&mut *state, IterState::Closed)
        };
        self.interrupt.store(true, Ordering::Release);
        if let IterState::Running(IterWorker {
            requests,
            results,
            handle,
            lease,
        }) = state
        {
            drop((requests, results));
            let result = join_worker(py, handle, &self.interrupt, None);
            lease.finish("cancelled");
            result?;
        }
        Ok(())
    }
}

impl<T, E> Drop for OwnedIter<T, E> {
    fn drop(&mut self) {
        self.interrupt.store(true, Ordering::Release);
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        if let IterState::Running(worker) = state {
            worker.lease.finish("cancelled");
        }
        // Disconnect without joining in a Python destructor. The Python-free worker owns
        // its repository and buffers and finishes native cleanup after seeing cancellation.
        *state = IterState::Closed;
    }
}

/// A persistent native owner uses the iterator transport to answer one command at a time.
pub struct CommandOwner<C, R> {
    commands: Arc<Mutex<Option<C>>>,
    executing: AtomicBool,
    inner: OwnedIter<PyResult<R>, PyErr>,
}

impl<C: Send + 'static, R: Send + 'static> CommandOwner<C, R> {
    pub fn new(
        name: &'static str,
        factory: impl FnOnce(OperationContext, Arc<Mutex<Option<C>>>, IterProducer<PyResult<R>, PyErr>) -> PyResult<()>
        + Send
        + 'static,
    ) -> Self {
        Self::new_with_options(name, None, None, factory)
    }
    pub fn new_with_options(
        name: &'static str,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
        factory: impl FnOnce(OperationContext, Arc<Mutex<Option<C>>>, IterProducer<PyResult<R>, PyErr>) -> PyResult<()>
        + Send
        + 'static,
    ) -> Self {
        let commands = Arc::new(Mutex::new(None));
        let incoming = commands.clone();
        Self {
            commands,
            executing: AtomicBool::new(false),
            inner: OwnedIter::new(name, progress, cancel, move |context, producer| {
                factory(context, incoming, producer)
            }),
        }
    }
    pub fn call(&self, py: Python<'_>, command: C) -> PyResult<R> {
        if self
            .executing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(PyRuntimeError::new_err("native handle is already executing"));
        }
        struct Release<'a>(&'a AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _release = Release(&self.executing);
        *self.commands.lock().unwrap_or_else(|e| e.into_inner()) = Some(command);
        self.inner
            .next(py)?
            .ok_or_else(|| PyRuntimeError::new_err("native handle is closed"))??
    }
    pub fn finish_initialization(&mut self) {
        self.inner.finish_initialization();
    }
    pub fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.inner.close(py)
    }
    pub fn finish(&self, py: Python<'_>) -> PyResult<()> {
        match self.inner.next(py)? {
            None => Ok(()),
            Some(Err(error)) | Some(Ok(Err(error))) => Err(error),
            Some(Ok(Ok(_))) => Err(PyRuntimeError::new_err(
                "native handle returned an unexpected final result",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn producer_advances_only_on_demand_and_closes_on_disconnect() {
        let advanced = Arc::new(AtomicUsize::new(0));
        let observed = advanced.clone();
        let (requests, receiver) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(1);
        let producer = IterProducer {
            requests: receiver,
            results: sender,
            interrupt: Arc::new(AtomicBool::new(false)),
        };
        let handle = thread::spawn(move || {
            producer.serve((0..3).map(|item| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(item)
            }))
        });
        assert_eq!(advanced.load(Ordering::SeqCst), 0);
        assert!(requests.send(()).is_ok());
        assert!(matches!(results.recv(), Ok(IterEvent::Item(0))));
        assert_eq!(advanced.load(Ordering::SeqCst), 1);
        drop(requests);
        assert!(matches!(handle.join(), Ok(Ok(()))));
        assert_eq!(advanced.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unused_iterator_does_not_construct_its_native_owner() {
        let constructed = Arc::new(AtomicBool::new(false));
        let observed = constructed.clone();
        let iter = OwnedIter::<(), ()>::new("test", None, None, move |_, producer| {
            observed.store(true, Ordering::SeqCst);
            producer.serve(std::iter::empty())
        });
        drop(iter);
        assert!(!constructed.load(Ordering::SeqCst));
    }

    #[test]
    fn cancellation_tokens_are_monotonic_and_shared() {
        let token = CancellationToken::default();
        let shared = token.clone();
        assert!(!shared.cancelled());
        token.cancel();
        shared.cancel();
        assert!(token.cancelled());
        assert!(shared.cancelled());
    }

    #[test]
    fn cancellation_joins_the_worker_and_updates_progress() {
        Python::initialize();
        Python::attach(|py| {
            let token = CancellationToken::default();
            let remote_token = token.clone();
            let finished = Arc::new(AtomicBool::new(false));
            let observed = finished.clone();
            let (started, ready) = mpsc::sync_channel(1);
            let canceller = thread::spawn(move || {
                assert!(ready.recv().is_ok());
                remote_token.cancel();
            });
            let progress = Progress::default();
            let result = run(py, "cancel test", Some(&progress), Some(&token), move |context| {
                assert!(started.send(()).is_ok());
                while !context.interrupt.load(Ordering::Acquire) {
                    thread::yield_now();
                }
                observed.store(true, Ordering::Release);
                Ok::<_, ()>(())
            });
            assert!(matches!(result, Err(error) if error.is_instance_of::<CancelledError>(py)));
            assert!(finished.load(Ordering::Acquire));
            assert_eq!(progress.state(), "cancelled");
            assert!(canceller.join().is_ok());
        });
    }

    #[test]
    fn cancelled_parked_iterator_does_not_advance_again() {
        Python::initialize();
        Python::attach(|py| {
            let token = CancellationToken::default();
            let advanced = Arc::new(AtomicUsize::new(0));
            let observed = advanced.clone();
            let iter = OwnedIter::new("parked", None, Some(&token), move |_, producer| {
                producer.serve((0..3).map(move |value| {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ()>(value)
                }))
            });
            assert!(matches!(iter.next(py), Ok(Some(Ok(0)))));
            token.cancel();
            assert!(matches!(iter.next(py), Err(e) if e.is_instance_of::<CancelledError>(py)));
            assert_eq!(advanced.load(Ordering::SeqCst), 1);
            assert!(matches!(iter.next(py), Ok(None)));
        });
    }

    #[test]
    fn owned_iterator_closes_cleanly_after_an_item() {
        Python::initialize();
        Python::attach(|py| {
            let progress = Progress::default();
            let iter = OwnedIter::new("items", Some(&progress), None, |_, producer| {
                producer.serve((0..3).map(Ok::<_, ()>))
            });
            assert_eq!(progress.state(), "idle");
            assert!(matches!(iter.next(py), Ok(Some(Ok(0)))));
            assert_eq!(progress.state(), "running");
            assert!(iter.close(py).is_ok());
            assert_eq!(progress.state(), "cancelled");
            assert!(matches!(iter.next(py), Ok(None)));
        });
    }
}
