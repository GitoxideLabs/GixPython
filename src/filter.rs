//! One native owner retains filter processes and lends one conversion stream at a time.

use std::{
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::UNIX_EPOCH,
};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};

use crate::{
    error::to_py,
    index::{IndexFile, IndexSnapshot},
    repository::Repository,
    runtime::{CancellationToken, CommandOwner, IterProducer, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};

type Owner = CommandOwner<Command, Reply>;
type Commands = Arc<Mutex<Option<Command>>>;
type Producer = IterProducer<PyResult<Reply>, PyErr>;

enum Command {
    Ready,
    ToGit(Vec<u8>, PathBuf, IndexSnapshot, Arc<AtomicBool>),
    ToWorktree(
        Vec<u8>,
        Vec<u8>,
        gix::filter::plumbing::pipeline::convert::to_worktree::UnknownEncoding,
        Arc<AtomicBool>,
    ),
    WorktreeFile(Vec<u8>, IndexSnapshot),
    Read(u64, Option<usize>),
    CloseRead(u64),
    GetContext,
    SetRefName(Option<Vec<u8>>),
    SetTreeish(Option<ObjectSpec>),
    SetBlob(Option<ObjectSpec>),
}

enum Reply {
    Index(IndexFile),
    Stream(u64, bool),
    Data(Vec<u8>, bool),
    File(Option<(ObjectId, String, FileMetadata)>),
    Context(gix::filter::plumbing::pipeline::Context),
    Unit,
}

fn unexpected() -> PyErr {
    PyRuntimeError::new_err("unexpected native filter reply")
}

fn take_command(commands: &Commands) -> PyResult<Command> {
    commands
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .ok_or_else(|| PyRuntimeError::new_err("filter command is missing"))
}

// A dropped Python reader releases its loan on the next command. No Python is
// called by a Rust destructor, and a kept reader prevents accidental reuse.
fn serve_reader(
    reader: &mut impl Read,
    id: u64,
    alive: &AtomicBool,
    commands: &Commands,
    producer: &Producer,
    interrupt: &AtomicBool,
) -> PyResult<Option<Command>> {
    while producer.requested() {
        let command = take_command(commands)?;
        match command {
            Command::Read(read_id, size) if read_id == id => {
                let result = read(reader, size, interrupt).map(|(data, eof)| Reply::Data(data, eof));
                let finished = matches!(result, Ok(Reply::Data(_, true)) | Err(_));
                if !producer.send(result) || finished {
                    return Ok(None);
                }
            }
            Command::CloseRead(read_id) if read_id == id => {
                producer.send(Ok(Reply::Unit));
                return Ok(None);
            }
            command if !alive.load(Ordering::Acquire) => return Ok(Some(command)),
            _ => {
                if !producer.send(Err(PyRuntimeError::new_err(
                    "close or exhaust the active filter stream before reusing its pipeline",
                ))) {
                    return Ok(None);
                }
            }
        }
    }
    Ok(None)
}

fn read(reader: &mut impl Read, size: Option<usize>, interrupt: &AtomicBool) -> PyResult<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    while size.is_none_or(|limit| out.len() < limit) {
        if interrupt.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err("filter read was interrupted"));
        }
        let count = size.map_or(buffer.len(), |limit| (limit - out.len()).min(buffer.len()));
        let count = reader.read(&mut buffer[..count]).map_err(to_py)?;
        if count == 0 {
            return Ok((out, true));
        }
        out.extend_from_slice(&buffer[..count]);
    }
    Ok((out, false))
}

#[pyclass(frozen, module = "gix")]
pub struct FileMetadata {
    inner: std::fs::Metadata,
}

#[pymethods]
impl FileMetadata {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }
    fn is_symlink(&self) -> bool {
        self.inner.is_symlink()
    }
    fn readonly(&self) -> bool {
        self.inner.permissions().readonly()
    }
    fn modified(&self) -> PyResult<f64> {
        timestamp(self.inner.modified().map_err(to_py)?)
    }
    fn accessed(&self) -> PyResult<f64> {
        timestamp(self.inner.accessed().map_err(to_py)?)
    }
    fn created(&self) -> PyResult<f64> {
        timestamp(self.inner.created().map_err(to_py)?)
    }
}

fn timestamp(value: std::time::SystemTime) -> PyResult<f64> {
    Ok(match value.duration_since(UNIX_EPOCH) {
        Ok(value) => value.as_secs_f64(),
        Err(value) => -value.duration().as_secs_f64(),
    })
}

#[pyclass(frozen, module = "gix")]
pub struct FilterPipeline {
    owner: Arc<Owner>,
}

impl FilterPipeline {
    fn start(&self, py: Python<'_>, command: Command, alive: Arc<AtomicBool>) -> PyResult<FilterRead> {
        match self.owner.call(py, command)? {
            Reply::Stream(id, changed) => Ok(FilterRead {
                owner: self.owner.clone(),
                id,
                changed,
                alive,
                closed: AtomicBool::new(false),
                exhausted: AtomicBool::new(false),
            }),
            _ => Err(unexpected()),
        }
    }
}

#[pymethods]
impl FilterPipeline {
    fn convert_to_git(
        &self,
        py: Python<'_>,
        src: &[u8],
        rela_path: PathBuf,
        index: &IndexFile,
    ) -> PyResult<FilterRead> {
        let alive = Arc::new(AtomicBool::new(true));
        self.start(
            py,
            Command::ToGit(src.to_vec(), rela_path, index.snapshot()?, alive.clone()),
            alive,
        )
    }
    #[pyo3(signature = (src, rela_path, *, unknown_encoding="ignore"))]
    fn convert_to_worktree(
        &self,
        py: Python<'_>,
        src: &[u8],
        rela_path: &Bound<'_, PyAny>,
        unknown_encoding: &str,
    ) -> PyResult<FilterRead> {
        use gix::filter::plumbing::pipeline::convert::to_worktree::UnknownEncoding;
        let encoding = match unknown_encoding {
            "ignore" => UnknownEncoding::Ignore,
            "fail" => UnknownEncoding::Fail,
            _ => return Err(PyValueError::new_err("unknown_encoding must be ignore or fail")),
        };
        let alive = Arc::new(AtomicBool::new(true));
        self.start(
            py,
            Command::ToWorktree(src.to_vec(), bytes(rela_path)?, encoding, alive.clone()),
            alive,
        )
    }
    fn worktree_file_to_object(
        &self,
        py: Python<'_>,
        rela_path: &Bound<'_, PyAny>,
        index: &IndexFile,
    ) -> PyResult<Option<(ObjectId, String, FileMetadata)>> {
        match self
            .owner
            .call(py, Command::WorktreeFile(bytes(rela_path)?, index.snapshot()?))?
        {
            Reply::File(value) => Ok(value),
            _ => Err(unexpected()),
        }
    }
    fn driver_context_mut(&self) -> FilterDriverContext {
        FilterDriverContext {
            owner: self.owner.clone(),
        }
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.owner.close(py)
    }
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.owner.close(py)
    }
}

#[pyclass(frozen, module = "gix")]
pub struct FilterRead {
    owner: Arc<Owner>,
    id: u64,
    changed: bool,
    alive: Arc<AtomicBool>,
    closed: AtomicBool,
    exhausted: AtomicBool,
}

impl Drop for FilterRead {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[pymethods]
impl FilterRead {
    #[pyo3(signature = (size=-1))]
    fn read<'py>(&self, py: Python<'py>, size: isize) -> PyResult<Bound<'py, PyBytes>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PyValueError::new_err("read from a closed filter stream"));
        }
        if self.exhausted.load(Ordering::Acquire) || size == 0 {
            return Ok(PyBytes::new(py, &[]));
        }
        match self
            .owner
            .call(py, Command::Read(self.id, (size >= 0).then_some(size as usize)))?
        {
            Reply::Data(data, eof) => {
                if eof {
                    self.exhausted.store(true, Ordering::Release);
                }
                Ok(PyBytes::new(py, &data))
            }
            _ => Err(unexpected()),
        }
    }
    fn is_changed(&self) -> bool {
        self.changed
    }
    fn is_delayed(&self) -> bool {
        false
    }
    #[getter]
    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        if !self.closed.load(Ordering::Acquire) {
            if !self.exhausted.load(Ordering::Acquire) {
                self.owner.call(py, Command::CloseRead(self.id))?;
            }
            self.closed.store(true, Ordering::Release);
            self.alive.store(false, Ordering::Release);
        }
        Ok(())
    }
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}

#[pyclass(frozen, module = "gix")]
pub struct FilterDriverContext {
    owner: Arc<Owner>,
}

impl FilterDriverContext {
    fn get(&self, py: Python<'_>) -> PyResult<gix::filter::plumbing::pipeline::Context> {
        match self.owner.call(py, Command::GetContext)? {
            Reply::Context(value) => Ok(value),
            _ => Err(unexpected()),
        }
    }
}

#[pymethods]
impl FilterDriverContext {
    #[getter]
    fn ref_name<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        Ok(self.get(py)?.ref_name.map(|value| PyBytes::new(py, &value)))
    }
    #[setter]
    fn set_ref_name(&self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.owner
            .call(py, Command::SetRefName(value.map(bytes).transpose()?))?;
        Ok(())
    }
    #[getter]
    fn treeish(&self, py: Python<'_>) -> PyResult<Option<ObjectId>> {
        Ok(self.get(py)?.treeish.map(|inner| ObjectId { inner }))
    }
    #[setter]
    fn set_treeish(&self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.owner
            .call(py, Command::SetTreeish(value.map(ObjectSpec::extract).transpose()?))?;
        Ok(())
    }
    #[getter]
    fn blob(&self, py: Python<'_>) -> PyResult<Option<ObjectId>> {
        Ok(self.get(py)?.blob.map(|inner| ObjectId { inner }))
    }
    #[setter]
    fn set_blob(&self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.owner
            .call(py, Command::SetBlob(value.map(ObjectSpec::extract).transpose()?))?;
        Ok(())
    }
}

#[pymethods]
impl Repository {
    #[pyo3(signature = (tree_if_bare=None, *, progress=None, cancel=None))]
    fn filter_pipeline(
        &self,
        py: Python<'_>,
        tree_if_bare: Option<&Bound<'_, PyAny>>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<(FilterPipeline, IndexFile)> {
        let tree = tree_if_bare.map(ObjectSpec::extract).transpose()?;
        let handle = self.handle.clone();
        let owner = Owner::new_with_options(
            "filter pipeline",
            progress,
            cancel,
            move |context, commands, producer| {
                handle.with(|repo| {
                    let tree = tree.map(|value| value.resolve(repo)).transpose()?;
                    let (mut pipeline, index) = repo.filter_pipeline(tree).map_err(to_py)?;
                    let index = match index {
                        gix::worktree::IndexPersistedOrInMemory::Persisted(index) => IndexFile::from_shared(index),
                        gix::worktree::IndexPersistedOrInMemory::InMemory(index) => IndexFile::from_native(index),
                    };
                    let mut pending = None;
                    let mut next_id = 0;
                    loop {
                        let command = match pending.take() {
                            Some(command) => command,
                            None => {
                                if !producer.requested() {
                                    break;
                                }
                                take_command(&commands)?
                            }
                        };
                        let reply = match command {
                            Command::Ready => Ok(Reply::Index(index.clone())),
                            Command::ToGit(src, path, index, alive) => {
                                match pipeline.convert_to_git(src.as_slice(), &path, &index) {
                                    Ok(mut reader) => {
                                        next_id += 1;
                                        if !producer.send(Ok(Reply::Stream(next_id, reader.is_changed()))) {
                                            break;
                                        }
                                        pending = serve_reader(
                                            &mut reader,
                                            next_id,
                                            &alive,
                                            &commands,
                                            &producer,
                                            &context.interrupt,
                                        )?;
                                        continue;
                                    }
                                    Err(error) => Err(to_py(error)),
                                }
                            }
                            Command::ToWorktree(src, path, unknown_encoding, alive) => {
                                let options = gix::filter::plumbing::pipeline::convert::to_worktree::Options {
                                    unknown_encoding,
                                    ..Default::default()
                                };
                                match pipeline.convert_to_worktree(&src, path.as_bstr(), options) {
                                    Ok(mut reader) => {
                                        next_id += 1;
                                        if !producer.send(Ok(Reply::Stream(next_id, reader.is_changed()))) {
                                            break;
                                        }
                                        pending = serve_reader(
                                            &mut reader,
                                            next_id,
                                            &alive,
                                            &commands,
                                            &producer,
                                            &context.interrupt,
                                        )?;
                                        continue;
                                    }
                                    Err(error) => Err(to_py(error)),
                                }
                            }
                            Command::WorktreeFile(path, index) => pipeline
                                .worktree_file_to_object(path.as_bstr(), &index)
                                .map(|value| {
                                    Reply::File(value.map(|(inner, kind, metadata)| {
                                        (
                                            ObjectId { inner },
                                            format!("{kind:?}"),
                                            FileMetadata { inner: metadata },
                                        )
                                    }))
                                })
                                .map_err(to_py),
                            Command::GetContext => Ok(Reply::Context(pipeline.driver_context_mut().clone())),
                            Command::SetRefName(value) => {
                                pipeline.driver_context_mut().ref_name = value.map(Into::into);
                                Ok(Reply::Unit)
                            }
                            Command::SetTreeish(value) => {
                                value.map(|value| value.resolve(repo)).transpose().map(|value| {
                                    pipeline.driver_context_mut().treeish = value;
                                    Reply::Unit
                                })
                            }
                            Command::SetBlob(value) => {
                                value.map(|value| value.resolve(repo)).transpose().map(|value| {
                                    pipeline.driver_context_mut().blob = value;
                                    Reply::Unit
                                })
                            }
                            Command::CloseRead(_) => Ok(Reply::Unit),
                            Command::Read(_, _) => Err(PyRuntimeError::new_err("filter stream is no longer active")),
                        };
                        if !producer.send(reply) {
                            break;
                        }
                    }
                    Ok(())
                })
            },
        );
        let index = match owner.call(py, Command::Ready)? {
            Reply::Index(index) => index,
            _ => return Err(unexpected()),
        };
        Ok((FilterPipeline { owner: Arc::new(owner) }, index))
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FilterPipeline>()?;
    m.add_class::<FilterRead>()?;
    m.add_class::<FilterDriverContext>()?;
    m.add_class::<FileMetadata>()?;
    Ok(())
}
