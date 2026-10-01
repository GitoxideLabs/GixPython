//! A native stream and its borrowed entry stay on one worker stack.

use crate::{
    error::to_py,
    index::IndexFile,
    repository::{RepoHandle, Repository},
    runtime::{CancellationToken, CommandOwner, IterProducer, Progress},
    types::{HashKind, ObjectId, ObjectSpec, bytes},
};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

type Owner = CommandOwner<Command, Reply>;
#[derive(Clone)]
struct EntryInfo {
    generation: u64,
    id: gix::ObjectId,
    mode: u16,
    path: Vec<u8>,
}
enum Command {
    Index,
    Next,
    Read(u64, Option<usize>),
    Remaining(u64),
    Add(AdditionalEntry),
    AddPath(PathBuf, PathBuf, gix::hash::Kind),
    #[cfg(feature = "worktree-archive")]
    Archive(PathBuf, ArchiveOptions),
}
enum Reply {
    Index(IndexFile),
    Entry(Option<EntryInfo>),
    Data(Vec<u8>),
    Remaining(Option<usize>),
    Done,
}
fn wrong_reply() -> PyErr {
    PyValueError::new_err("unexpected native stream response")
}
fn invalid_entry() -> PyErr {
    PyValueError::new_err("stream entry has been closed or invalidated by advancing the stream")
}
fn next_command(
    producer: &IterProducer<PyResult<Reply>, PyErr>,
    commands: &Arc<Mutex<Option<Command>>>,
) -> Option<Command> {
    if !producer.requested() {
        return None;
    }
    commands.lock().unwrap_or_else(|e| e.into_inner()).take()
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct StreamSource {
    inner: Source,
}
#[derive(Clone)]
enum Source {
    Null,
    Memory(Vec<u8>),
    Path(PathBuf),
}
#[pymethods]
impl StreamSource {
    #[classattr]
    #[pyo3(name = "Null")]
    fn null() -> Self {
        Self { inner: Source::Null }
    }
    #[staticmethod]
    #[pyo3(name = "Memory")]
    fn memory(data: &Bound<'_, PyBytes>) -> Self {
        Self {
            inner: Source::Memory(data.as_bytes().to_vec()),
        }
    }
    #[staticmethod]
    #[pyo3(name = "Path")]
    fn path(path: PathBuf) -> Self {
        Self {
            inner: Source::Path(path),
        }
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct AdditionalEntry {
    id: gix::ObjectId,
    mode: gix::objs::tree::EntryMode,
    path: Vec<u8>,
    source: Source,
}
#[pymethods]
impl AdditionalEntry {
    #[new]
    fn new(id: ObjectId, mode: u16, relative_path: &Bound<'_, PyAny>, source: StreamSource) -> PyResult<Self> {
        Ok(Self {
            id: id.inner,
            mode: u32::from(mode)
                .try_into()
                .map_err(|_| PyValueError::new_err("invalid Git tree entry mode"))?,
            path: bytes(relative_path)?,
            source: source.inner,
        })
    }
}
impl AdditionalEntry {
    fn into_native(self) -> gix::worktree::stream::AdditionalEntry {
        gix::worktree::stream::AdditionalEntry {
            id: self.id,
            mode: self.mode,
            relative_path: self.path.into(),
            source: match self.source {
                Source::Null => gix::worktree::stream::entry::Source::Null,
                Source::Memory(data) => gix::worktree::stream::entry::Source::Memory(data),
                Source::Path(path) => gix::worktree::stream::entry::Source::Path(path),
            },
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct WorktreeStream {
    owner: Arc<Owner>,
    generation: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    exhausted: AtomicBool,
}
#[pyclass(frozen, module = "gix")]
pub struct WorktreeStreamEntry {
    owner: Weak<Owner>,
    generation: Arc<AtomicU64>,
    stream_closed: Arc<AtomicBool>,
    closed: AtomicBool,
    info: EntryInfo,
}
impl WorktreeStreamEntry {
    fn check(&self) -> PyResult<()> {
        if self.closed.load(Ordering::Acquire)
            || self.stream_closed.load(Ordering::Acquire)
            || self.info.generation != self.generation.load(Ordering::Acquire)
        {
            Err(invalid_entry())
        } else {
            Ok(())
        }
    }
}
#[pymethods]
impl WorktreeStreamEntry {
    #[getter]
    fn id(&self) -> ObjectId {
        ObjectId { inner: self.info.id }
    }
    #[getter]
    fn mode(&self) -> u16 {
        self.info.mode
    }
    fn relative_path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.info.path)
    }
    fn bytes_remaining(&self, py: Python<'_>) -> PyResult<Option<usize>> {
        self.check()?;
        match self
            .owner
            .upgrade()
            .ok_or_else(invalid_entry)?
            .call(py, Command::Remaining(self.info.generation))?
        {
            Reply::Remaining(value) => Ok(value),
            _ => Err(wrong_reply()),
        }
    }
    #[pyo3(signature=(size=-1))]
    fn read<'py>(&self, py: Python<'py>, size: isize) -> PyResult<Bound<'py, PyBytes>> {
        self.check()?;
        if size == 0 {
            return Ok(PyBytes::new(py, b""));
        }
        let limit = (size >= 0).then_some(size as usize);
        match self
            .owner
            .upgrade()
            .ok_or_else(invalid_entry)?
            .call(py, Command::Read(self.info.generation, limit))?
        {
            Reply::Data(data) => Ok(PyBytes::new(py, &data)),
            _ => Err(wrong_reply()),
        }
    }
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        slf.check()?;
        Ok(slf)
    }
    fn __exit__(&self, _exc_type: &Bound<'_, PyAny>, _exc_value: &Bound<'_, PyAny>, _traceback: &Bound<'_, PyAny>) {
        self.close()
    }
}
#[pymethods]
impl WorktreeStream {
    fn next_entry(&self, py: Python<'_>) -> PyResult<Option<WorktreeStreamEntry>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PyValueError::new_err("worktree stream is closed"));
        }
        if self.exhausted.load(Ordering::Acquire) {
            return Ok(None);
        }
        match self.owner.call(py, Command::Next)? {
            Reply::Entry(None) => {
                self.exhausted.store(true, Ordering::Release);
                self.owner.finish(py)?;
                Ok(None)
            }
            Reply::Entry(info) => Ok(info.map(|info| WorktreeStreamEntry {
                owner: Arc::downgrade(&self.owner),
                generation: self.generation.clone(),
                stream_closed: self.closed.clone(),
                closed: AtomicBool::new(false),
                info,
            })),
            _ => Err(wrong_reply()),
        }
    }
    fn add_entry(&self, py: Python<'_>, entry: AdditionalEntry) -> PyResult<()> {
        match self.owner.call(py, Command::Add(entry))? {
            Reply::Done => Ok(()),
            _ => Err(wrong_reply()),
        }
    }
    fn add_entry_from_path(&self, py: Python<'_>, root: PathBuf, path: PathBuf, object_hash: HashKind) -> PyResult<()> {
        match self.owner.call(py, Command::AddPath(root, path, object_hash.inner))? {
            Reply::Done => Ok(()),
            _ => Err(wrong_reply()),
        }
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.closed.store(true, Ordering::Release);
        if self.exhausted.load(Ordering::Acquire) {
            Ok(())
        } else {
            self.owner.close(py)
        }
    }
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}
impl Drop for WorktreeStream {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

fn create_stream(
    py: Python<'_>,
    handle: RepoHandle,
    id: ObjectSpec,
    progress: Option<&Progress>,
    cancel: Option<&CancellationToken>,
) -> PyResult<(WorktreeStream, IndexFile)> {
    let generation = Arc::new(AtomicU64::new(0));
    let native_generation = generation.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let owner = Arc::new(Owner::new_with_options(
        "worktree stream",
        progress,
        cancel,
        move |mut context, commands, producer| {
            handle.with(|repo| {
                context.progress.init(None, gix::progress::count("entries"));
                let (mut native, index) = repo.worktree_stream(id.resolve(repo)?).map_err(to_py)?;
                let mut index = Some(index);
                while let Some(command) = next_command(&producer, &commands) {
                    match command {
                        Command::Index => {
                            if !producer.send(
                                index
                                    .take()
                                    .map(|index| Reply::Index(IndexFile::from_native(index)))
                                    .ok_or_else(wrong_reply),
                            ) {
                                break;
                            }
                        }
                        Command::Add(entry) => {
                            native.add_entry(entry.into_native());
                            let result = Ok(Reply::Done);
                            if !producer.send(result) {
                                break;
                            }
                        }
                        Command::AddPath(root, path, hash) => {
                            let result = native
                                .add_entry_from_path(&root, &path, hash)
                                .map(|_| Reply::Done)
                                .map_err(to_py);
                            if !producer.send(result) {
                                break;
                            }
                        }
                        Command::Next => 'entries: loop {
                            let current = native_generation.fetch_add(1, Ordering::AcqRel) + 1;
                            let Some(mut entry) = native.next_entry().map_err(to_py)? else {
                                producer.send(Ok(Reply::Entry(None)));
                                return Ok(());
                            };
                            let info = EntryInfo {
                                generation: current,
                                id: entry.id,
                                mode: entry.mode.value(),
                                path: entry.relative_path().to_vec(),
                            };
                            context.progress.inc();
                            if !producer.send(Ok(Reply::Entry(Some(info)))) {
                                return Ok(());
                            }
                            loop {
                                let Some(command) = next_command(&producer, &commands) else {
                                    return Ok(());
                                };
                                match command {
                                    Command::Read(generation, limit) => {
                                        if generation != current {
                                            if !producer.send(Err(invalid_entry())) {
                                                return Ok(());
                                            }
                                            continue;
                                        }
                                        let mut data = Vec::new();
                                        let mut interruptible = gix::features::interrupt::Read {
                                            inner: &mut entry,
                                            should_interrupt: &context.interrupt,
                                        };
                                        if let Some(limit) = limit {
                                            interruptible
                                                .by_ref()
                                                .take(limit as u64)
                                                .read_to_end(&mut data)
                                                .map_err(to_py)?;
                                        } else {
                                            interruptible.read_to_end(&mut data).map_err(to_py)?;
                                        }
                                        if !producer.send(Ok(Reply::Data(data))) {
                                            return Ok(());
                                        }
                                    }
                                    Command::Remaining(generation) => {
                                        if !producer.send(if generation == current {
                                            Ok(Reply::Remaining(entry.bytes_remaining()))
                                        } else {
                                            Err(invalid_entry())
                                        }) {
                                            return Ok(());
                                        }
                                    }
                                    Command::Next => {
                                        native_generation.fetch_add(1, Ordering::AcqRel);
                                        std::io::copy(
                                            &mut gix::features::interrupt::Read {
                                                inner: &mut entry,
                                                should_interrupt: &context.interrupt,
                                            },
                                            &mut std::io::sink(),
                                        )
                                        .map_err(to_py)?;
                                        drop(entry);
                                        continue 'entries;
                                    }
                                    _ => {
                                        if !producer.send(Err(PyValueError::new_err(
                                            "operation requires an unstarted worktree stream",
                                        ))) {
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        },
                        Command::Read(_, _) | Command::Remaining(_) => {
                            if !producer.send(Err(invalid_entry())) {
                                break;
                            }
                        }
                        #[cfg(feature = "worktree-archive")]
                        Command::Archive(path, options) => {
                            let mut out = std::io::BufWriter::new(std::fs::File::create(path).map_err(to_py)?);
                            repo.worktree_archive(
                                native,
                                &mut out,
                                &mut context.progress,
                                &context.interrupt,
                                options.inner,
                            )
                            .map_err(to_py)?;
                            out.flush().map_err(to_py)?;
                            producer.send(Ok(Reply::Done));
                            return Ok(());
                        }
                    }
                }
                Ok(())
            })
        },
    ));
    match owner.call(py, Command::Index)? {
        Reply::Index(index) => Ok((
            WorktreeStream {
                owner,
                generation,
                closed,
                exhausted: AtomicBool::new(false),
            },
            index,
        )),
        _ => Err(wrong_reply()),
    }
}
#[pymethods]
impl Repository {
    #[pyo3(signature=(id,*,progress=None,cancel=None))]
    fn worktree_stream(
        &self,
        py: Python<'_>,
        id: &Bound<'_, PyAny>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<(WorktreeStream, IndexFile)> {
        create_stream(py, self.handle.clone(), ObjectSpec::extract(id)?, progress, cancel)
    }
}

#[cfg(feature = "worktree-archive")]
#[pyclass(module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct ArchiveOptions {
    inner: gix::worktree::archive::Options,
}
#[cfg(feature = "worktree-archive")]
#[pymethods]
impl ArchiveOptions {
    #[new]
    #[pyo3(signature=(format="internal",*,compression_level=None,tree_prefix=None,modification_time=None))]
    fn new(
        format: &str,
        compression_level: Option<u8>,
        tree_prefix: Option<&Bound<'_, PyAny>>,
        modification_time: Option<i64>,
    ) -> PyResult<Self> {
        if compression_level.is_some_and(|level| level > 9) {
            return Err(PyValueError::new_err("compression level must be 0 through 9"));
        }
        let format = match format {
            "tar" if cfg!(feature = "archive-tar") => gix::worktree::archive::Format::Tar,
            "tar.gz" if cfg!(feature = "archive-tar-gz") => gix::worktree::archive::Format::TarGz { compression_level },
            "zip" if cfg!(feature = "archive-zip") => gix::worktree::archive::Format::Zip { compression_level },
            "internal" => gix::worktree::archive::Format::InternalTransientNonPersistable,
            _ => {
                return Err(PyValueError::new_err(
                    "archive format is unknown or disabled in this build",
                ));
            }
        };
        let mut inner = gix::worktree::archive::Options {
            format,
            tree_prefix: tree_prefix.map(bytes).transpose()?.map(Into::into),
            ..Default::default()
        };
        if let Some(time) = modification_time {
            inner.modification_time = time;
        }
        Ok(Self { inner })
    }
}
#[cfg(feature = "worktree-archive")]
#[pymethods]
impl Repository {
    #[pyo3(signature=(stream,out,options=None))]
    fn worktree_archive(
        &self,
        py: Python<'_>,
        stream: &WorktreeStream,
        out: PathBuf,
        options: Option<ArchiveOptions>,
    ) -> PyResult<()> {
        let options = options.unwrap_or_default();
        match stream.owner.call(py, Command::Archive(out, options))? {
            Reply::Done => {
                stream.exhausted.store(true, Ordering::Release);
                stream.closed.store(true, Ordering::Release);
                stream.owner.finish(py)
            }
            _ => Err(wrong_reply()),
        }
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<StreamSource>()?;
    m.add_class::<AdditionalEntry>()?;
    m.add_class::<WorktreeStream>()?;
    m.add_class::<WorktreeStreamEntry>()?;
    #[cfg(feature = "worktree-archive")]
    m.add_class::<ArchiveOptions>()?;
    Ok(())
}
