//! Native tree changes and reusable blob diff resources.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyTuple},
};

use crate::{
    diff_options::Rewrites,
    error::to_py,
    objects::Tree,
    repository::{RepoHandle, Repository},
    runtime::{CancellationToken, OwnedIter, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};

fn exn_to_py<E: std::error::Error + Send + Sync + 'static>(error: gix::Exn<E>) -> PyErr {
    to_py(gix::Error::from(error))
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct DiffOptions {
    inner: gix::diff::Options,
}

#[pymethods]
impl DiffOptions {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn no_locations(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        slf.inner.no_locations();
        slf
    }
    fn track_filename(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        slf.inner.track_filename();
        slf
    }
    fn track_path(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        slf.inner.track_path();
        slf
    }
    fn track_rewrites(mut slf: PyRefMut<'_, Self>, rewrites: Option<Rewrites>) -> PyRefMut<'_, Self> {
        slf.inner.track_rewrites(rewrites.map(|v| v.inner));
        slf
    }
    fn with_rewrites(&self, rewrites: Option<Rewrites>) -> Self {
        Self {
            inner: self.inner.with_rewrites(rewrites.map(|v| v.inner)),
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct DiffLineStats {
    pub inner: gix::diff::blob::DiffLineStats,
}

#[pymethods]
impl DiffLineStats {
    #[getter]
    fn removals(&self) -> u32 {
        self.inner.removals
    }
    #[getter]
    fn insertions(&self) -> u32 {
        self.inner.insertions
    }
    #[getter]
    fn before(&self) -> usize {
        self.inner.before
    }
    #[getter]
    fn after(&self) -> usize {
        self.inner.after
    }
    #[getter]
    fn similarity(&self) -> f32 {
        self.inner.similarity
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeChange {
    pub inner: gix::object::tree::diff::ChangeDetached,
}

#[pymethods]
impl TreeChange {
    #[getter]
    fn kind(&self) -> &'static str {
        use gix::object::tree::diff::ChangeDetached as Change;
        match self.inner {
            Change::Addition { .. } => "Addition",
            Change::Deletion { .. } => "Deletion",
            Change::Modification { .. } => "Modification",
            Change::Rewrite { .. } => "Rewrite",
        }
    }
    fn location<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.location())
    }
    fn source_location<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.source_location())
    }
    fn id(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.entry_mode_and_id().1.to_owned(),
        }
    }
    fn entry_mode(&self) -> u32 {
        self.inner.entry_mode().value().into()
    }
    #[getter]
    fn details<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        use gix::object::tree::diff::ChangeDetached as Change;
        let out = PyDict::new(py);
        out.set_item("kind", self.kind())?;
        out.set_item("location", self.location(py))?;
        out.set_item("entry_mode", self.entry_mode())?;
        out.set_item("id", self.id())?;
        match &self.inner {
            Change::Addition { relation, .. } | Change::Deletion { relation, .. } => {
                out.set_item("relation", relation.map(|v| format!("{v:?}")))?;
            }
            Change::Modification {
                previous_entry_mode,
                previous_id,
                ..
            } => {
                out.set_item("previous_entry_mode", previous_entry_mode.value())?;
                out.set_item("previous_id", ObjectId { inner: *previous_id })?;
            }
            Change::Rewrite {
                source_location,
                source_relation,
                source_entry_mode,
                source_id,
                diff,
                relation,
                copy,
                ..
            } => {
                out.set_item("source_location", PyBytes::new(py, source_location))?;
                out.set_item("source_relation", source_relation.map(|v| format!("{v:?}")))?;
                out.set_item("source_entry_mode", source_entry_mode.value())?;
                out.set_item("source_id", ObjectId { inner: *source_id })?;
                out.set_item("diff", diff.map(|inner| DiffLineStats { inner }))?;
                out.set_item("relation", relation.map(|v| format!("{v:?}")))?;
                out.set_item("copy", copy)?;
            }
        }
        Ok(out)
    }
    fn diff(&self, py: Python<'_>, resource_cache: DiffResourceCache) -> PyResult<BlobDiff> {
        let change = self.inner.clone();
        let command = change.clone();
        resource_cache.with(py, move |cache, repo| {
            cache
                .set_resource_by_change(command.to_ref(), &repo.objects)
                .map(|_| ())
                .map_err(exn_to_py)
        })?;
        Ok(BlobDiff {
            cache: resource_cache,
            change,
        })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct DiffResourceCache {
    handle: RepoHandle,
    inner: Arc<Mutex<gix::diff::blob::Platform>>,
}

impl DiffResourceCache {
    fn with<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::diff::blob::Platform, &gix::Repository) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let cache = self.inner.clone();
        self.handle.run(py, move |repo| {
            let mut cache = cache
                .try_lock()
                .map_err(|_| PyRuntimeError::new_err("diff resource cache is already in use"))?;
            work(&mut cache, repo)
        })
    }
}

fn resource_kind(kind: &str) -> PyResult<gix::diff::blob::ResourceKind> {
    match kind {
        "old" => Ok(gix::diff::blob::ResourceKind::OldOrSource),
        "new" => Ok(gix::diff::blob::ResourceKind::NewOrDestination),
        _ => Err(PyValueError::new_err("resource kind must be old or new")),
    }
}

#[pymethods]
impl DiffResourceCache {
    fn set_resource(
        &self,
        py: Python<'_>,
        id: &Bound<'_, PyAny>,
        mode: u32,
        rela_path: &Bound<'_, PyAny>,
        kind: &str,
    ) -> PyResult<()> {
        let id = ObjectSpec::extract(id)?;
        let path = bytes(rela_path)?;
        let kind = resource_kind(kind)?;
        let mode = gix::objs::tree::EntryMode::try_from(mode)
            .map_err(|_| PyValueError::new_err("invalid entry mode"))?
            .kind();
        self.with(py, move |cache, repo| {
            cache
                .set_resource(id.resolve(repo)?, mode, path.as_bstr(), kind, &repo.objects)
                .map_err(exn_to_py)
        })
    }
    fn resource(&self, py: Python<'_>, kind: &str) -> PyResult<Option<DiffResource>> {
        let kind = resource_kind(kind)?;
        self.with(py, move |cache, _| {
            Ok(cache.resource(kind).map(DiffResource::from_native))
        })
    }
    fn prepare_diff(&self, py: Python<'_>) -> PyResult<PreparedDiff> {
        self.with(py, |cache, _| {
            cache.prepare_diff().map(PreparedDiff::from_native).map_err(exn_to_py)
        })
    }
    fn clear_resource_cache(&self, py: Python<'_>) -> PyResult<()> {
        self.with(py, |cache, _| {
            cache.clear_resource_cache();
            Ok(())
        })
    }
    fn clear_resource_cache_keep_allocation(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, |cache, _| Ok(cache.clear_resource_cache_keep_allocation()))
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct DiffResource {
    #[pyo3(get)]
    pub id: ObjectId,
    path: Vec<u8>,
    data: Option<Vec<u8>>,
    #[pyo3(get)]
    pub mode: u32,
    #[pyo3(get)]
    pub driver_index: Option<usize>,
    #[pyo3(get)]
    pub is_derived: bool,
    #[pyo3(get)]
    pub kind: &'static str,
    #[pyo3(get)]
    pub binary_size: Option<u64>,
}

impl DiffResource {
    fn from_native(resource: gix::diff::blob::platform::Resource<'_>) -> Self {
        use gix::diff::blob::platform::resource::Data;
        let (kind, binary_size) = match resource.data {
            Data::Missing => ("Missing", None),
            Data::Buffer { .. } => ("Buffer", None),
            Data::Binary { size } => ("Binary", Some(size)),
        };
        Self {
            id: ObjectId {
                inner: resource.id.to_owned(),
            },
            path: resource.rela_path.to_vec(),
            data: resource.data.as_slice().map(|v| v.to_vec()),
            mode: resource.mode as u32,
            driver_index: resource.driver_index,
            is_derived: resource.data.is_derived(),
            kind,
            binary_size,
        }
    }
}

#[pymethods]
impl DiffResource {
    #[getter]
    fn rela_path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.path)
    }
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.data.as_ref().map(|v| PyBytes::new(py, v))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct PreparedDiff {
    #[pyo3(get)]
    old: DiffResource,
    #[pyo3(get)]
    new: DiffResource,
    #[pyo3(get)]
    operation: &'static str,
    #[pyo3(get)]
    algorithm: Option<String>,
    command: Option<Vec<u8>>,
    #[pyo3(get)]
    old_or_new_is_derived: bool,
}

impl PreparedDiff {
    fn from_native(out: gix::diff::blob::platform::prepare_diff::Outcome<'_>) -> Self {
        use gix::diff::blob::platform::prepare_diff::Operation;
        let (operation, algorithm, command) = match out.operation {
            Operation::InternalDiff { algorithm } => ("InternalDiff", Some(format!("{algorithm:?}")), None),
            Operation::ExternalCommand { command } => ("ExternalCommand", None, Some(command.to_vec())),
            Operation::SourceOrDestinationIsBinary => ("SourceOrDestinationIsBinary", None, None),
        };
        Self {
            old: DiffResource::from_native(out.old),
            new: DiffResource::from_native(out.new),
            operation,
            algorithm,
            command,
            old_or_new_is_derived: out.old_or_new_is_derived,
        }
    }
}

#[pymethods]
impl PreparedDiff {
    #[getter]
    fn command<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.command.as_ref().map(|v| PyBytes::new(py, v))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct BlobDiff {
    cache: DiffResourceCache,
    change: gix::object::tree::diff::ChangeDetached,
}

#[pyclass(frozen, module = "gix")]
pub struct DiffHunk {
    #[pyo3(get)]
    before: (u32, u32),
    #[pyo3(get)]
    after: (u32, u32),
    lines_before: Vec<Vec<u8>>,
    lines_after: Vec<Vec<u8>>,
}

#[pymethods]
impl DiffHunk {
    #[getter]
    fn lines_before<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.lines_before.iter().map(|v| PyBytes::new(py, v)))
    }
    #[getter]
    fn lines_after<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.lines_after.iter().map(|v| PyBytes::new(py, v)))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct DiffHunks {
    inner: OwnedIter<DiffHunk, PyErr>,
}

#[pymethods]
impl DiffHunks {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<DiffHunk>> {
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
impl BlobDiff {
    fn line_counts(&self, py: Python<'_>) -> PyResult<Option<DiffLineStats>> {
        let change = self.change.clone();
        self.cache.with(py, move |cache, repo| {
            cache
                .set_resource_by_change(change.to_ref(), &repo.objects)
                .map_err(exn_to_py)?;
            gix::object::blob::diff::Platform { resource_cache: cache }
                .line_counts()
                .map(|v| v.map(|inner| DiffLineStats { inner }))
                .map_err(to_py)
        })
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn lines(&self, progress: Option<&Progress>, cancel: Option<&CancellationToken>) -> DiffHunks {
        let cache = self.cache.clone();
        let change = self.change.clone();
        DiffHunks {
            inner: OwnedIter::new("diff lines", progress, cancel, move |_, producer| {
                cache.handle.with(|repo| {
                    let mut cache = cache
                        .inner
                        .try_lock()
                        .map_err(|_| PyRuntimeError::new_err("diff resource cache is already in use"))?;
                    cache
                        .set_resource_by_change(change.to_ref(), &repo.objects)
                        .map_err(exn_to_py)?;
                    cache.options.skip_internal_diff_if_external_is_configured = false;
                    let prepared = cache.prepare_diff().map_err(exn_to_py)?;
                    if let gix::diff::blob::platform::prepare_diff::Operation::InternalDiff { algorithm } =
                        prepared.operation
                    {
                        let input = prepared.interned_input();
                        let diff = gix::diff::blob::diff_with_slider_heuristics(algorithm, &input);
                        producer.serve(diff.hunks().map(|hunk| {
                            Ok(DiffHunk {
                                before: (hunk.before.start, hunk.before.end),
                                after: (hunk.after.start, hunk.after.end),
                                lines_before: input.before[hunk.before.start as usize..hunk.before.end as usize]
                                    .iter()
                                    .map(|&token| input.interner[token].to_vec())
                                    .collect(),
                                lines_after: input.after[hunk.after.start as usize..hunk.after.end as usize]
                                    .iter()
                                    .map(|&token| input.interner[token].to_vec())
                                    .collect(),
                            })
                        }))
                    } else {
                        producer.serve(std::iter::empty())
                    }
                })
            }),
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeDiff {
    tree: Tree,
    options: Option<gix::diff::Options>,
}

#[pyclass(frozen, module = "gix")]
pub struct DiffStats {
    #[pyo3(get)]
    files_changed: u64,
    #[pyo3(get)]
    lines_added: u64,
    #[pyo3(get)]
    lines_removed: u64,
}

#[pymethods]
impl TreeDiff {
    fn options(&self, options: DiffOptions) -> Self {
        Self {
            tree: self.tree.clone(),
            options: Some(options.inner),
        }
    }
    #[pyo3(signature = (other, *, progress=None, cancel=None))]
    fn for_each_to_obtain_tree(
        &self,
        other: &Tree,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> TreeChanges {
        self.iter(other, None, progress, cancel)
    }
    #[pyo3(signature = (other, resource_cache, *, progress=None, cancel=None))]
    fn for_each_to_obtain_tree_with_cache(
        &self,
        other: &Tree,
        resource_cache: DiffResourceCache,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> TreeChanges {
        self.iter(other, Some(resource_cache), progress, cancel)
    }
    fn stats(&self, py: Python<'_>, other: &Tree) -> PyResult<DiffStats> {
        let old = self.tree.object.inner.clone();
        let new = other.object.inner.clone();
        let options = self.options;
        self.tree.object.handle.run(py, move |repo| {
            let old = gix::Tree::from_data(old.id, old.data.clone(), repo);
            let new = gix::Tree::from_data(new.id, new.data.clone(), repo);
            let mut changes = old.changes().map_err(to_py)?;
            if let Some(options) = options {
                changes.options(|opts| {
                    *opts = options;
                });
            }
            let stats = changes.stats(&new).map_err(to_py)?;
            Ok(DiffStats {
                files_changed: stats.files_changed,
                lines_added: stats.lines_added,
                lines_removed: stats.lines_removed,
            })
        })
    }
}

impl TreeDiff {
    fn iter(
        &self,
        other: &Tree,
        cache: Option<DiffResourceCache>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> TreeChanges {
        let old = self.tree.object.inner.clone();
        let new = other.object.inner.clone();
        let handle = self.tree.object.handle.clone();
        let options = self.options;
        TreeChanges {
            inner: OwnedIter::new("tree changes", progress, cancel, move |_, producer| {
                if !producer.requested() {
                    return Ok(());
                }
                handle.with(|repo| {
                    if old.id.kind() != new.id.kind() {
                        return Err(PyValueError::new_err("tree hash kinds differ"));
                    }
                    let old = gix::Tree::from_data(old.id, old.data.clone(), repo);
                    let new = gix::Tree::from_data(new.id, new.data.clone(), repo);
                    let mut changes = old.changes().map_err(to_py)?;
                    if let Some(options) = options {
                        changes.options(|opts| {
                            *opts = options;
                        });
                    }
                    let callback = |change: gix::object::tree::diff::Change<'_, '_, '_>| {
                        let keep_going = producer.send(TreeChange { inner: change.detach() }) && producer.requested();
                        Ok(if keep_going {
                            std::ops::ControlFlow::Continue(())
                        } else {
                            std::ops::ControlFlow::Break(())
                        })
                    };
                    if let Some(cache) = cache {
                        let mut cache = cache
                            .inner
                            .try_lock()
                            .map_err(|_| PyRuntimeError::new_err("diff resource cache is already in use"))?;
                        changes
                            .for_each_to_obtain_tree_with_cache(&new, &mut cache, callback)
                            .map_err(to_py)?;
                    } else {
                        changes.for_each_to_obtain_tree(&new, callback).map_err(to_py)?;
                    }
                    Ok(())
                })
            }),
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeChanges {
    inner: OwnedIter<TreeChange, PyErr>,
}

#[pymethods]
impl TreeChanges {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<TreeChange>> {
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

fn diff_trees(
    py: Python<'_>,
    handle: &RepoHandle,
    old: Option<Tree>,
    new: Option<Tree>,
    options: Option<gix::diff::Options>,
) -> PyResult<Vec<TreeChange>> {
    handle.run(py, move |repo| {
        for tree in [&old, &new].into_iter().flatten() {
            if tree.object.inner.id.kind() != repo.object_hash() {
                return Err(PyValueError::new_err("tree hash kind differs from repository"));
            }
        }
        let old = old.map(|v| gix::Tree::from_data(v.object.inner.id, v.object.inner.data.clone(), repo));
        let new = new.map(|v| gix::Tree::from_data(v.object.inner.id, v.object.inner.data.clone(), repo));
        repo.diff_tree_to_tree(old.as_ref(), new.as_ref(), options)
            .map(|changes| changes.into_iter().map(|inner| TreeChange { inner }).collect())
            .map_err(to_py)
    })
}

#[pymethods]
impl Tree {
    fn changes(&self) -> TreeDiff {
        TreeDiff {
            tree: self.clone(),
            options: None,
        }
    }
}

#[pymethods]
impl Repository {
    #[pyo3(signature = (old_tree=None, new_tree=None, options=None))]
    fn diff_tree_to_tree(
        &self,
        py: Python<'_>,
        old_tree: Option<Tree>,
        new_tree: Option<Tree>,
        options: Option<DiffOptions>,
    ) -> PyResult<Vec<TreeChange>> {
        diff_trees(py, &self.handle, old_tree, new_tree, options.map(|v| v.inner))
    }
    #[pyo3(signature = (mode="to_git", *, old_root=None, new_root=None))]
    fn diff_resource_cache(
        &self,
        py: Python<'_>,
        mode: &str,
        old_root: Option<PathBuf>,
        new_root: Option<PathBuf>,
    ) -> PyResult<DiffResourceCache> {
        use gix::diff::blob::pipeline::Mode;
        let mode = match mode {
            "to_git" => Mode::ToGit,
            "to_worktree_and_binary_to_text" => Mode::ToWorktreeAndBinaryToText,
            "to_git_unless_binary_to_text_is_present" => Mode::ToGitUnlessBinaryToTextIsPresent,
            _ => return Err(PyValueError::new_err("unknown diff resource mode")),
        };
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            repo.diff_resource_cache(mode, gix::diff::blob::pipeline::WorktreeRoots { old_root, new_root })
                .map(|cache| DiffResourceCache {
                    handle,
                    inner: Arc::new(Mutex::new(cache)),
                })
                .map_err(to_py)
        })
    }
    fn diff_resource_cache_for_tree_diff(&self, py: Python<'_>) -> PyResult<DiffResourceCache> {
        self.diff_resource_cache(py, "to_git", None, None)
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DiffOptions>()?;
    m.add_class::<TreeChange>()?;
    m.add_class::<TreeDiff>()?;
    m.add_class::<TreeChanges>()?;
    m.add_class::<DiffResourceCache>()?;
    m.add_class::<DiffResource>()?;
    m.add_class::<PreparedDiff>()?;
    m.add_class::<BlobDiff>()?;
    m.add_class::<DiffHunk>()?;
    m.add_class::<DiffHunks>()?;
    m.add_class::<DiffLineStats>()?;
    m.add_class::<DiffStats>()?;
    Ok(())
}
