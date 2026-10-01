//! Native index snapshots with copy-on-write mutation and lazy entry views.

use std::{
    ops::Deref,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyIndexError, PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};

use crate::{
    error::to_py,
    repository::Repository,
    runtime::{self, CancellationToken, OwnedIter, Progress},
    types::{HashKind, ObjectId, ObjectSpec, bytes},
};

#[derive(Clone)]
pub enum IndexSnapshot {
    Shared(gix::worktree::Index),
    Owned(Arc<gix::index::File>),
}

impl Deref for IndexSnapshot {
    type Target = gix::index::File;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(index) => index,
            Self::Owned(index) => index,
        }
    }
}

impl IndexSnapshot {
    #[cfg(feature = "status")]
    pub fn into_owned(self) -> gix::index::File {
        match self {
            Self::Shared(index) => gix::index::File::clone(&index),
            Self::Owned(index) => Arc::unwrap_or_clone(index),
        }
    }
    fn make_mut(&mut self) -> &mut gix::index::File {
        if let Self::Shared(index) = self {
            *self = Self::Owned(Arc::new(gix::index::File::clone(index)));
        }
        match self {
            Self::Owned(index) => Arc::make_mut(index),
            Self::Shared(_) => unreachable!("converted above"),
        }
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct IndexFile {
    inner: Arc<RwLock<IndexSnapshot>>,
}

impl IndexFile {
    pub fn from_native(index: gix::index::File) -> Self {
        Self {
            inner: Arc::new(RwLock::new(IndexSnapshot::Owned(Arc::new(index)))),
        }
    }
    pub fn from_shared(index: gix::worktree::Index) -> Self {
        Self {
            inner: Arc::new(RwLock::new(IndexSnapshot::Shared(index))),
        }
    }
    pub fn snapshot(&self) -> PyResult<IndexSnapshot> {
        // Shared read locks only cover cloning ownership, so independent readers
        // overlap without waiting for a mutation while attached to Python.
        self.inner
            .try_read()
            .map(|index| index.clone())
            .map_err(|_| PyRuntimeError::new_err("index is already being mutated"))
    }
    pub(crate) fn mutate<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::index::File) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let inner = self.inner.clone();
        runtime::run(py, "index mutation", None, None, move |_| {
            let mut state = inner
                .try_write()
                .map_err(|_| PyRuntimeError::new_err("index is already being mutated"))?;
            work(state.make_mut())
        })?
    }
}

fn stage(value: u32) -> PyResult<gix::index::entry::Stage> {
    use gix::index::entry::Stage;
    match value {
        0 => Ok(Stage::Unconflicted),
        1 => Ok(Stage::Base),
        2 => Ok(Stage::Ours),
        3 => Ok(Stage::Theirs),
        _ => Err(PyValueError::new_err("stage must be 0, 1, 2, or 3")),
    }
}

fn mode(value: u32) -> PyResult<gix::index::entry::Mode> {
    let value = gix::index::entry::Mode::from_bits_retain(value);
    if value.to_tree_entry_mode().is_none() {
        return Err(PyValueError::new_err("invalid index entry mode"));
    }
    Ok(value)
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Copy, Default)]
pub struct IndexStat {
    pub inner: gix::index::entry::Stat,
}

#[pymethods]
impl IndexStat {
    #[new]
    #[pyo3(signature = (*, mtime=(0, 0), ctime=(0, 0), dev=0, ino=0, uid=0, gid=0, size=0))]
    fn new(mtime: (u32, u32), ctime: (u32, u32), dev: u32, ino: u32, uid: u32, gid: u32, size: u32) -> Self {
        Self {
            inner: gix::index::entry::Stat {
                mtime: gix::index::entry::stat::Time {
                    secs: mtime.0,
                    nsecs: mtime.1,
                },
                ctime: gix::index::entry::stat::Time {
                    secs: ctime.0,
                    nsecs: ctime.1,
                },
                dev,
                ino,
                uid,
                gid,
                size,
            },
        }
    }
    #[getter]
    fn mtime(&self) -> (u32, u32) {
        (self.inner.mtime.secs, self.inner.mtime.nsecs)
    }
    #[getter]
    fn ctime(&self) -> (u32, u32) {
        (self.inner.ctime.secs, self.inner.ctime.nsecs)
    }
    #[getter]
    fn dev(&self) -> u32 {
        self.inner.dev
    }
    #[getter]
    fn ino(&self) -> u32 {
        self.inner.ino
    }
    #[getter]
    fn uid(&self) -> u32 {
        self.inner.uid
    }
    #[getter]
    fn gid(&self) -> u32 {
        self.inner.gid
    }
    #[getter]
    fn size(&self) -> u32 {
        self.inner.size
    }
}

#[pyclass(frozen, module = "gix")]
pub struct IndexEntry {
    pub inner: gix::index::Entry,
    pub path: Vec<u8>,
}

impl IndexEntry {
    pub fn from_native(entry: &gix::index::Entry, index: &gix::index::State) -> Self {
        Self {
            inner: entry.clone(),
            path: entry.path(index).to_vec(),
        }
    }
}

#[pymethods]
impl IndexEntry {
    #[getter]
    fn id(&self) -> ObjectId {
        ObjectId { inner: self.inner.id }
    }
    #[getter]
    fn mode(&self) -> u32 {
        self.inner.mode.bits()
    }
    #[getter]
    fn flags(&self) -> u32 {
        self.inner.flags.bits()
    }
    #[getter]
    fn stat(&self) -> IndexStat {
        IndexStat { inner: self.inner.stat }
    }
    fn path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.path)
    }
    fn stage(&self) -> u32 {
        self.inner.stage_raw()
    }
    fn stage_raw(&self) -> u32 {
        self.inner.stage_raw()
    }
}

#[pyclass(frozen, module = "gix")]
pub struct IndexEntryMut {
    index: IndexFile,
    path: Vec<u8>,
    stage: gix::index::entry::Stage,
}

impl IndexEntryMut {
    fn read(&self) -> PyResult<gix::index::Entry> {
        self.index
            .snapshot()?
            .entry_by_path_and_stage(self.path.as_bstr(), self.stage)
            .cloned()
            .ok_or_else(|| PyKeyError::new_err("index entry no longer exists"))
    }
    fn mutate(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::index::Entry) -> PyResult<()> + Send + 'static,
    ) -> PyResult<()> {
        let path = self.path.clone();
        let stage = self.stage;
        self.index.mutate(py, move |index| {
            let entry = index
                .entry_mut_by_path_and_stage(path.as_bstr(), stage)
                .ok_or_else(|| PyKeyError::new_err("index entry no longer exists"))?;
            work(entry)?;
            // gix currently writes tree caches unchanged; invalidate after entry edits.
            index.remove_tree();
            Ok(())
        })
    }
}

#[pymethods]
impl IndexEntryMut {
    #[getter]
    fn id(&self) -> PyResult<ObjectId> {
        Ok(ObjectId { inner: self.read()?.id })
    }
    #[setter]
    fn set_id(&self, py: Python<'_>, value: ObjectId) -> PyResult<()> {
        if value.inner.kind() != self.index.snapshot()?.object_hash() {
            return Err(PyValueError::new_err("object ID hash kind differs from the index"));
        }
        self.mutate(py, move |entry| {
            entry.id = value.inner;
            Ok(())
        })
    }
    #[getter]
    fn mode(&self) -> PyResult<u32> {
        Ok(self.read()?.mode.bits())
    }
    #[setter]
    fn set_mode(&self, py: Python<'_>, value: u32) -> PyResult<()> {
        let mode = mode(value)?;
        self.mutate(py, move |entry| {
            entry.mode = mode;
            Ok(())
        })
    }
    #[getter]
    fn flags(&self) -> PyResult<u32> {
        Ok(self.read()?.flags.bits())
    }
    #[setter]
    fn set_flags(&self, py: Python<'_>, value: u32) -> PyResult<()> {
        let flags = gix::index::entry::Flags::from_bits_retain(value);
        if flags.stage() != self.stage {
            return Err(PyValueError::new_err(
                "changing the stage would invalidate this mutable entry view",
            ));
        }
        self.mutate(py, move |entry| {
            entry.flags = flags;
            Ok(())
        })
    }
    #[getter]
    fn stat(&self) -> PyResult<IndexStat> {
        Ok(IndexStat {
            inner: self.read()?.stat,
        })
    }
    #[setter]
    fn set_stat(&self, py: Python<'_>, value: IndexStat) -> PyResult<()> {
        self.mutate(py, move |entry| {
            entry.stat = value.inner;
            Ok(())
        })
    }
    fn path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.path)
    }
    fn stage(&self) -> u32 {
        self.stage as u32
    }
    fn stage_raw(&self) -> u32 {
        self.stage as u32
    }
}

#[pyclass(frozen, module = "gix")]
pub struct IndexEntries {
    inner: OwnedIter<IndexEntry, PyErr>,
}

#[pymethods]
impl IndexEntries {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<IndexEntry>> {
        self.inner.next(py)?.transpose()
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.inner.close(py)
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
        self.inner.close(py)
    }
}

#[pymethods]
impl IndexFile {
    fn path(&self) -> PyResult<PathBuf> {
        Ok(self.snapshot()?.path().into())
    }
    fn set_path(&self, py: Python<'_>, path: PathBuf) -> PyResult<()> {
        self.mutate(py, move |index| {
            index.set_path(path);
            Ok(())
        })
    }
    fn object_hash(&self) -> PyResult<HashKind> {
        Ok(HashKind {
            inner: self.snapshot()?.object_hash(),
        })
    }
    fn version(&self) -> PyResult<u32> {
        Ok(self.snapshot()?.version() as u32)
    }
    fn timestamp(&self) -> PyResult<(i64, u32)> {
        let t = self.snapshot()?.timestamp();
        Ok((t.unix_seconds(), t.nanoseconds()))
    }
    fn checksum(&self) -> PyResult<Option<ObjectId>> {
        Ok(self.snapshot()?.checksum().map(|inner| ObjectId { inner }))
    }
    fn is_sparse(&self) -> PyResult<bool> {
        Ok(self.snapshot()?.is_sparse())
    }
    fn __len__(&self) -> PyResult<usize> {
        Ok(self.snapshot()?.entries().len())
    }
    fn __iter__(&self) -> PyResult<IndexEntries> {
        self.entries(None, None)
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn entries(&self, progress: Option<&Progress>, cancel: Option<&CancellationToken>) -> PyResult<IndexEntries> {
        let index = self.snapshot()?;
        Ok(IndexEntries {
            inner: OwnedIter::new("index entries", progress, cancel, move |_, producer| {
                producer.serve(
                    index
                        .entries()
                        .iter()
                        .map(|entry| Ok(IndexEntry::from_native(entry, &index))),
                )
            }),
        })
    }
    fn entry(&self, position: usize) -> PyResult<IndexEntry> {
        let index = self.snapshot()?;
        index
            .entries()
            .get(position)
            .map(|entry| IndexEntry::from_native(entry, &index))
            .ok_or_else(|| PyIndexError::new_err("index entry out of range"))
    }
    fn entry_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Option<IndexEntry>> {
        let index = self.snapshot()?;
        Ok(index
            .entry_by_path(bytes(path)?.as_bstr())
            .map(|entry| IndexEntry::from_native(entry, &index)))
    }
    fn entry_by_path_and_stage(&self, path: &Bound<'_, PyAny>, entry_stage: u32) -> PyResult<Option<IndexEntry>> {
        let index = self.snapshot()?;
        Ok(index
            .entry_by_path_and_stage(bytes(path)?.as_bstr(), stage(entry_stage)?)
            .map(|entry| IndexEntry::from_native(entry, &index)))
    }
    fn entry_mut_by_path_and_stage(
        &self,
        path: &Bound<'_, PyAny>,
        entry_stage: u32,
    ) -> PyResult<Option<IndexEntryMut>> {
        let path = bytes(path)?;
        let stage = stage(entry_stage)?;
        Ok(self
            .snapshot()?
            .entry_by_path_and_stage(path.as_bstr(), stage)
            .map(|_| IndexEntryMut {
                index: self.clone(),
                path,
                stage,
            }))
    }
    fn entry_index_by_path_and_stage(&self, path: &Bound<'_, PyAny>, entry_stage: u32) -> PyResult<Option<usize>> {
        Ok(self
            .snapshot()?
            .entry_index_by_path_and_stage(bytes(path)?.as_bstr(), stage(entry_stage)?))
    }
    fn entry_range(&self, path: &Bound<'_, PyAny>) -> PyResult<Option<(usize, usize)>> {
        Ok(self
            .snapshot()?
            .entry_range(bytes(path)?.as_bstr())
            .map(|r| (r.start, r.end)))
    }
    fn prefixed_entries_range(&self, prefix: &Bound<'_, PyAny>) -> PyResult<Option<(usize, usize)>> {
        Ok(self
            .snapshot()?
            .prefixed_entries_range(bytes(prefix)?.as_bstr())
            .map(|r| (r.start, r.end)))
    }
    fn path_is_directory(&self, path: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.snapshot()?.path_is_directory(bytes(path)?.as_bstr()))
    }
    fn dangerously_push_entry(
        &self,
        py: Python<'_>,
        stat: IndexStat,
        id: ObjectId,
        flags: u32,
        entry_mode: u32,
        path: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let path = bytes(path)?;
        let mode = mode(entry_mode)?;
        self.mutate(py, move |index| {
            if id.inner.kind() != index.object_hash() {
                return Err(PyValueError::new_err("object ID hash kind differs from the index"));
            }
            index.dangerously_push_entry(
                stat.inner,
                id.inner,
                gix::index::entry::Flags::from_bits_retain(flags),
                mode,
                path.as_bstr(),
            );
            index.remove_tree();
            Ok(())
        })
    }
    fn sort_entries(&self, py: Python<'_>) -> PyResult<()> {
        self.mutate(py, |index| {
            index.sort_entries();
            Ok(())
        })
    }
    fn remove_entry_at_index(&self, py: Python<'_>, position: usize) -> PyResult<IndexEntry> {
        self.mutate(py, move |index| {
            let entry = index
                .entries()
                .get(position)
                .ok_or_else(|| PyIndexError::new_err("index entry out of range"))?;
            let entry = IndexEntry::from_native(entry, index);
            index.remove_entry_at_index(position);
            index.remove_tree();
            Ok(entry)
        })
    }
    fn verify_entries(&self, py: Python<'_>) -> PyResult<()> {
        let index = self.snapshot()?;
        py.detach(move || index.verify_entries().map_err(to_py))
    }
    fn verify_integrity(&self, py: Python<'_>) -> PyResult<()> {
        let index = self.snapshot()?;
        runtime::run(py, "verify index", None, None, move |_| {
            index.verify_integrity().map_err(to_py)
        })?
    }
    #[pyo3(signature = (*, skip_hash=false, extensions="all"))]
    fn write(&self, py: Python<'_>, skip_hash: bool, extensions: &str) -> PyResult<()> {
        let extensions = match extensions {
            "all" => gix::index::write::Extensions::All,
            "none" => gix::index::write::Extensions::None,
            _ => return Err(PyValueError::new_err("extensions must be all or none")),
        };
        self.mutate(py, move |index| {
            index.verify_entries().map_err(to_py)?;
            index
                .write(gix::index::write::Options { skip_hash, extensions })
                .map_err(to_py)
        })
    }
}

#[pymethods]
impl Repository {
    fn open_index(&self, py: Python<'_>) -> PyResult<IndexFile> {
        self.handle
            .run(py, |repo| repo.open_index().map(IndexFile::from_native).map_err(to_py))
    }
    fn index(&self, py: Python<'_>) -> PyResult<IndexFile> {
        self.handle
            .run(py, |repo| repo.index().map(IndexFile::from_shared).map_err(to_py))
    }
    fn try_index(&self, py: Python<'_>) -> PyResult<Option<IndexFile>> {
        self.handle.run(py, |repo| {
            repo.try_index().map(|v| v.map(IndexFile::from_shared)).map_err(to_py)
        })
    }
    fn index_or_empty(&self, py: Python<'_>) -> PyResult<IndexFile> {
        self.handle.run(py, |repo| {
            repo.index_or_empty().map(IndexFile::from_shared).map_err(to_py)
        })
    }
    fn index_or_load_from_head(&self, py: Python<'_>) -> PyResult<IndexFile> {
        self.handle.run(py, |repo| {
            repo.index_or_load_from_head()
                .map(|index| match index {
                    gix::worktree::IndexPersistedOrInMemory::Persisted(index) => IndexFile::from_shared(index),
                    gix::worktree::IndexPersistedOrInMemory::InMemory(index) => IndexFile::from_native(index),
                })
                .map_err(to_py)
        })
    }
    fn index_or_load_from_head_or_empty(&self, py: Python<'_>) -> PyResult<IndexFile> {
        self.handle.run(py, |repo| {
            repo.index_or_load_from_head_or_empty()
                .map(|index| match index {
                    gix::worktree::IndexPersistedOrInMemory::Persisted(index) => IndexFile::from_shared(index),
                    gix::worktree::IndexPersistedOrInMemory::InMemory(index) => IndexFile::from_native(index),
                })
                .map_err(to_py)
        })
    }
    fn index_from_tree(&self, py: Python<'_>, tree: &Bound<'_, PyAny>) -> PyResult<IndexFile> {
        let tree = ObjectSpec::extract(tree)?;
        self.handle.run(py, move |repo| {
            repo.index_from_tree(&tree.resolve(repo)?)
                .map(IndexFile::from_native)
                .map_err(to_py)
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<IndexFile>()?;
    m.add_class::<IndexEntry>()?;
    m.add_class::<IndexEntryMut>()?;
    m.add_class::<IndexEntries>()?;
    m.add_class::<IndexStat>()?;
    Ok(())
}
