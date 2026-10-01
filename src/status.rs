//! Native status builders, lazy traversal, and writable status outcomes.

use std::sync::{Arc, Mutex};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
};

use crate::{
    diff_options::Rewrites,
    error::to_py,
    index::{IndexEntry, IndexFile, IndexSnapshot, IndexStat},
    repository::{RepoHandle, Repository},
    runtime::{self, CancellationToken, OwnedIter, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};

#[pyclass(frozen, module = "gix")]
pub struct SubmoduleStatus {
    pub inner: gix::submodule::Status,
}

#[pymethods]
impl SubmoduleStatus {
    fn is_dirty(&self) -> Option<bool> {
        self.inner.is_dirty()
    }
    #[getter]
    fn index_id(&self) -> Option<ObjectId> {
        self.inner.index_id.map(|inner| ObjectId { inner })
    }
    #[getter]
    fn checked_out_head_id(&self) -> Option<ObjectId> {
        self.inner.checked_out_head_id.map(|inner| ObjectId { inner })
    }
    #[getter]
    fn state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        out.set_item("repository_exists", self.inner.state.repository_exists)?;
        out.set_item("is_old_form", self.inner.state.is_old_form)?;
        out.set_item("worktree_checkout", self.inner.state.worktree_checkout)?;
        out.set_item(
            "superproject_configuration",
            self.inner.state.superproject_configuration,
        )?;
        Ok(out)
    }
    #[getter]
    fn changes(&self) -> Option<Vec<StatusItem>> {
        self.inner
            .changes
            .as_ref()
            .map(|changes| changes.iter().cloned().map(|inner| StatusItem { inner }).collect())
    }
}

#[pyclass(frozen, module = "gix")]
pub struct StatusItem {
    pub inner: gix::status::Item,
}

#[pymethods]
impl StatusItem {
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            gix::status::Item::IndexWorktree(_) => "IndexWorktree",
            gix::status::Item::TreeIndex(_) => "TreeIndex",
        }
    }
    fn location<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.location())
    }
    fn summary(&self) -> Option<String> {
        match &self.inner {
            gix::status::Item::IndexWorktree(item) => item.summary().map(|v| format!("{v:?}")),
            gix::status::Item::TreeIndex(item) => Some(
                match item {
                    gix::diff::index::Change::Addition { .. } => "Added",
                    gix::diff::index::Change::Deletion { .. } => "Removed",
                    gix::diff::index::Change::Modification { .. } => "Modified",
                    gix::diff::index::Change::Rewrite { copy, .. } => {
                        if *copy {
                            "Copied"
                        } else {
                            "Renamed"
                        }
                    }
                }
                .into(),
            ),
        }
    }
    #[getter]
    fn details<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        match &self.inner {
            gix::status::Item::IndexWorktree(item) => worktree_item(py, item),
            gix::status::Item::TreeIndex(item) => index_change(py, item),
        }
    }
}

fn index_change<'py>(py: Python<'py>, change: &gix::diff::index::Change) -> PyResult<Bound<'py, PyDict>> {
    use gix::diff::index::Change;
    let out = PyDict::new(py);
    let (location, index, mode, id) = change.fields();
    out.set_item("location", PyBytes::new(py, location))?;
    out.set_item("index", index)?;
    out.set_item("entry_mode", mode.bits())?;
    out.set_item("id", ObjectId { inner: id.to_owned() })?;
    out.set_item(
        "kind",
        match change {
            Change::Addition { .. } => "Addition",
            Change::Deletion { .. } => "Deletion",
            Change::Modification { .. } => "Modification",
            Change::Rewrite { .. } => "Rewrite",
        },
    )?;
    match change {
        Change::Modification {
            previous_index,
            previous_entry_mode,
            previous_id,
            ..
        } => {
            out.set_item("previous_index", previous_index)?;
            out.set_item("previous_entry_mode", previous_entry_mode.bits())?;
            out.set_item(
                "previous_id",
                ObjectId {
                    inner: previous_id.as_ref().to_owned(),
                },
            )?;
        }
        Change::Rewrite {
            source_location,
            source_index,
            source_entry_mode,
            source_id,
            copy,
            ..
        } => {
            out.set_item("source_location", PyBytes::new(py, source_location))?;
            out.set_item("source_index", source_index)?;
            out.set_item("source_entry_mode", source_entry_mode.bits())?;
            out.set_item(
                "source_id",
                ObjectId {
                    inner: source_id.as_ref().to_owned(),
                },
            )?;
            out.set_item("copy", copy)?;
        }
        _ => {}
    }
    Ok(out)
}

fn entry_status<'py>(
    py: Python<'py>,
    status: &gix::status::plumbing::index_as_worktree::EntryStatus<(), gix::submodule::Status>,
) -> PyResult<Bound<'py, PyDict>> {
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
    let out = PyDict::new(py);
    match status {
        EntryStatus::IntentToAdd => out.set_item("kind", "IntentToAdd")?,
        EntryStatus::NeedsUpdate(stat) => {
            out.set_item("kind", "NeedsUpdate")?;
            out.set_item("stat", IndexStat { inner: *stat })?;
        }
        EntryStatus::Conflict { summary, entries } => {
            out.set_item("kind", "Conflict")?;
            out.set_item("summary", format!("{summary:?}"))?;
            let entries = entries
                .iter()
                .map(|entry| -> PyResult<Option<Bound<'py, PyDict>>> {
                    entry
                        .as_ref()
                        .map(|entry| {
                            let out = PyDict::new(py);
                            out.set_item("id", ObjectId { inner: entry.id })?;
                            out.set_item("flags", entry.flags.bits())?;
                            out.set_item("mode", entry.mode.bits())?;
                            Ok(out)
                        })
                        .transpose()
                })
                .collect::<PyResult<Vec<_>>>()?;
            out.set_item("entries", entries)?;
        }
        EntryStatus::Change(change) => {
            out.set_item("kind", "Change")?;
            match change {
                Change::Removed => out.set_item("change", "Removed")?,
                Change::Type { worktree_mode } => {
                    out.set_item("change", "Type")?;
                    out.set_item("worktree_mode", worktree_mode.bits())?;
                }
                Change::Modification {
                    executable_bit_changed,
                    content_change,
                    set_entry_stat_size_zero,
                } => {
                    out.set_item("change", "Modification")?;
                    out.set_item("executable_bit_changed", executable_bit_changed)?;
                    out.set_item("content_changed", content_change.is_some())?;
                    out.set_item("set_entry_stat_size_zero", set_entry_stat_size_zero)?;
                }
                Change::SubmoduleModification(status) => {
                    out.set_item("change", "SubmoduleModification")?;
                    out.set_item("status", SubmoduleStatus { inner: status.clone() })?;
                }
            }
        }
    }
    Ok(out)
}

pub fn dir_entry<'py>(py: Python<'py>, entry: &gix::dir::Entry) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("rela_path", PyBytes::new(py, &entry.rela_path))?;
    out.set_item("status", format!("{:?}", entry.status))?;
    out.set_item("property", entry.property.map(|v| format!("{v:?}")))?;
    out.set_item("disk_kind", entry.disk_kind.map(|v| format!("{v:?}")))?;
    out.set_item("index_kind", entry.index_kind.map(|v| format!("{v:?}")))?;
    out.set_item("pathspec_match", entry.pathspec_match.map(|v| format!("{v:?}")))?;
    Ok(out)
}

fn worktree_item<'py>(py: Python<'py>, item: &gix::status::index_worktree::Item) -> PyResult<Bound<'py, PyDict>> {
    use gix::status::index_worktree::{Item, RewriteSource};
    let out = PyDict::new(py);
    match item {
        Item::Modification {
            entry,
            entry_index,
            rela_path,
            status,
        } => {
            out.set_item("kind", "Modification")?;
            out.set_item(
                "entry",
                IndexEntry {
                    inner: entry.clone(),
                    path: rela_path.to_vec(),
                },
            )?;
            out.set_item("entry_index", entry_index)?;
            out.set_item("rela_path", PyBytes::new(py, rela_path))?;
            out.set_item("status", entry_status(py, status)?)?;
        }
        Item::DirectoryContents {
            entry,
            collapsed_directory_status,
        } => {
            out.set_item("kind", "DirectoryContents")?;
            out.set_item("entry", dir_entry(py, entry)?)?;
            out.set_item(
                "collapsed_directory_status",
                collapsed_directory_status.map(|v| format!("{v:?}")),
            )?;
        }
        Item::Rewrite {
            source,
            dirwalk_entry,
            dirwalk_entry_collapsed_directory_status,
            dirwalk_entry_id,
            diff,
            copy,
        } => {
            out.set_item("kind", "Rewrite")?;
            out.set_item("dirwalk_entry", dir_entry(py, dirwalk_entry)?)?;
            out.set_item(
                "dirwalk_entry_collapsed_directory_status",
                dirwalk_entry_collapsed_directory_status.map(|v| format!("{v:?}")),
            )?;
            out.set_item(
                "dirwalk_entry_id",
                ObjectId {
                    inner: *dirwalk_entry_id,
                },
            )?;
            out.set_item("copy", copy)?;
            out.set_item(
                "diff",
                diff.as_ref()
                    .map(|v| (v.removals, v.insertions, v.before, v.after, v.similarity)),
            )?;
            let source_out = PyDict::new(py);
            match source {
                RewriteSource::RewriteFromIndex {
                    source_entry,
                    source_entry_index,
                    source_rela_path,
                    source_status,
                } => {
                    source_out.set_item("kind", "RewriteFromIndex")?;
                    source_out.set_item(
                        "source_entry",
                        IndexEntry {
                            inner: source_entry.clone(),
                            path: source_rela_path.to_vec(),
                        },
                    )?;
                    source_out.set_item("source_entry_index", source_entry_index)?;
                    source_out.set_item("source_rela_path", PyBytes::new(py, source_rela_path))?;
                    source_out.set_item("source_status", entry_status(py, source_status)?)?;
                }
                RewriteSource::CopyFromDirectoryEntry {
                    source_dirwalk_entry,
                    source_dirwalk_entry_collapsed_directory_status,
                    source_dirwalk_entry_id,
                } => {
                    source_out.set_item("kind", "CopyFromDirectoryEntry")?;
                    source_out.set_item("source_dirwalk_entry", dir_entry(py, source_dirwalk_entry)?)?;
                    source_out.set_item(
                        "source_dirwalk_entry_collapsed_directory_status",
                        source_dirwalk_entry_collapsed_directory_status.map(|v| format!("{v:?}")),
                    )?;
                    source_out.set_item(
                        "source_dirwalk_entry_id",
                        ObjectId {
                            inner: *source_dirwalk_entry_id,
                        },
                    )?;
                }
            }
            out.set_item("source", source_out)?;
        }
    }
    Ok(out)
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct StatusPlatform {
    handle: RepoHandle,
    progress: Option<Progress>,
    cancel: Option<CancellationToken>,
    untracked: Option<gix::status::UntrackedFiles>,
    index: Option<IndexSnapshot>,
    head: Option<gix::ObjectId>,
    rewrites: Option<Option<gix::diff::Rewrites>>,
    tree_renames: gix::status::tree_index::TrackRenames,
    submodules: Option<Option<gix::status::Submodule>>,
    thread_limit: Option<usize>,
    sorting: bool,
    ignored: bool,
}

#[pymethods]
impl StatusPlatform {
    fn untracked_files(&self, value: &str) -> PyResult<Self> {
        let value = match value {
            "none" => gix::status::UntrackedFiles::None,
            "collapsed" => gix::status::UntrackedFiles::Collapsed,
            "files" => gix::status::UntrackedFiles::Files,
            _ => {
                return Err(PyValueError::new_err(
                    "untracked files must be none, collapsed, or files",
                ));
            }
        };
        let mut out = self.clone();
        out.untracked = Some(value);
        Ok(out)
    }
    fn index(&self, index: &IndexFile) -> PyResult<Self> {
        let mut out = self.clone();
        out.index = Some(index.snapshot()?);
        Ok(out)
    }
    fn head_tree(&self, py: Python<'_>, tree: &Bound<'_, PyAny>) -> PyResult<Self> {
        let tree = ObjectSpec::extract(tree)?;
        let mut out = self.clone();
        out.head = Some(self.handle.run(py, move |repo| tree.resolve(repo))?);
        Ok(out)
    }
    fn index_worktree_rewrites(&self, rewrites: Option<Rewrites>) -> Self {
        let mut out = self.clone();
        out.rewrites = Some(rewrites.map(|v| v.inner));
        out
    }
    fn tree_index_track_renames(&self, rewrites: Option<Rewrites>) -> Self {
        let mut out = self.clone();
        out.tree_renames = rewrites.map_or(gix::status::tree_index::TrackRenames::Disabled, |v| {
            gix::status::tree_index::TrackRenames::Given(v.inner)
        });
        out
    }
    #[pyo3(signature = (ignore=None, *, check_dirty=false))]
    fn index_worktree_submodules(&self, ignore: Option<&str>, check_dirty: bool) -> PyResult<Self> {
        let mut out = self.clone();
        out.submodules = Some(match ignore {
            None => None,
            Some("configured") => Some(gix::status::Submodule::AsConfigured { check_dirty }),
            Some(value) => Some(gix::status::Submodule::Given {
                ignore: gix::submodule::config::Ignore::try_from(value.as_bytes().as_bstr())
                    .map_err(|_| PyValueError::new_err("ignore must be configured, all, dirty, untracked, or none"))?,
                check_dirty,
            }),
        });
        Ok(out)
    }
    #[pyo3(signature = (*, thread_limit=None, sorting=false))]
    fn index_worktree_options_mut(&self, thread_limit: Option<usize>, sorting: bool) -> Self {
        let mut out = self.clone();
        out.thread_limit = thread_limit;
        out.sorting = sorting;
        out
    }
    #[pyo3(signature = (*, emit_ignored=false))]
    fn dirwalk_options(&self, emit_ignored: bool) -> Self {
        let mut out = self.clone();
        out.ignored = emit_ignored;
        out
    }
    #[pyo3(signature = (patterns=Vec::new()))]
    fn into_iter(&self, patterns: Vec<Bound<'_, PyAny>>) -> PyResult<StatusIter> {
        self.iter(patterns, false)
    }
    #[pyo3(signature = (patterns=Vec::new()))]
    fn into_index_worktree_iter(&self, patterns: Vec<Bound<'_, PyAny>>) -> PyResult<StatusIter> {
        self.iter(patterns, true)
    }
}

impl StatusPlatform {
    fn iter(&self, patterns: Vec<Bound<'_, PyAny>>, worktree_only: bool) -> PyResult<StatusIter> {
        let patterns = patterns
            .iter()
            .map(|value| bytes(value).map(gix::bstr::BString::from))
            .collect::<PyResult<Vec<_>>>()?;
        let settings = self.clone();
        let outcome = Arc::new(Mutex::new(None));
        let output = outcome.clone();
        let inner = OwnedIter::new(
            "status",
            self.progress.as_ref(),
            self.cancel.as_ref(),
            move |context, producer| {
                settings.handle.with(|repo| {
                    let mut platform = repo
                        .status(context.progress)
                        .map_err(to_py)?
                        .should_interrupt_owned(context.interrupt)
                        .tree_index_track_renames(settings.tree_renames);
                    if let Some(untracked) = settings.untracked {
                        platform = platform.untracked_files(untracked);
                    }
                    if let Some(index) = settings.index {
                        platform =
                            platform.index(gix::worktree::IndexPersistedOrInMemory::InMemory(index.into_owned()));
                    }
                    if let Some(head) = settings.head {
                        platform = platform.head_tree(head);
                    }
                    if let Some(rewrites) = settings.rewrites {
                        platform = platform.index_worktree_rewrites(rewrites);
                    }
                    if let Some(submodules) = settings.submodules {
                        platform = platform.index_worktree_submodules(submodules);
                    }
                    platform = platform.index_worktree_options_mut(|options| {
                        options.thread_limit = settings.thread_limit;
                        if settings.sorting {
                            options.sorting = Some(
                                gix::status::plumbing::index_as_worktree_with_renames::Sorting::ByPathCaseSensitive,
                            );
                        }
                    });
                    if settings.ignored {
                        platform = platform.dirwalk_options(|options| {
                            options.emit_ignored(Some(gix::dir::walk::EmissionMode::Matching))
                        });
                    }
                    let result = if worktree_only {
                        let mut iter = platform.into_index_worktree_iter(patterns).map_err(to_py)?;
                        producer.serve(iter.by_ref().map(|v| {
                            v.map(|inner| StatusItem {
                                inner: gix::status::Item::IndexWorktree(inner),
                            })
                            .map_err(to_py)
                        }))?;
                        iter.into_outcome()
                    } else {
                        let mut iter = platform.into_iter(patterns).map_err(to_py)?;
                        producer.serve(
                            iter.by_ref()
                                .map(|v| v.map(|inner| StatusItem { inner }).map_err(to_py)),
                        )?;
                        iter.into_outcome()
                    };
                    *output.lock().unwrap_or_else(|e| e.into_inner()) = result;
                    Ok(())
                })
            },
        );
        Ok(StatusIter { inner, outcome })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct StatusIter {
    inner: OwnedIter<StatusItem, PyErr>,
    outcome: Arc<Mutex<Option<gix::status::Outcome>>>,
}

#[pymethods]
impl StatusIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<StatusItem>> {
        self.inner.next(py)?.transpose()
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.inner.close(py)
    }
    fn outcome_mut(&self) -> PyResult<Option<StatusOutcome>> {
        Ok(self
            .outcome
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("status outcome is in use"))?
            .is_some()
            .then(|| StatusOutcome {
                inner: self.outcome.clone(),
            }))
    }
    fn into_outcome(&self, py: Python<'_>) -> PyResult<Option<StatusOutcome>> {
        self.inner.close(py)?;
        Ok(self
            .outcome
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("status outcome is in use"))?
            .take()
            .map(|outcome| StatusOutcome {
                inner: Arc::new(Mutex::new(Some(outcome))),
            }))
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

#[pyclass(frozen, module = "gix")]
pub struct StatusOutcome {
    inner: Arc<Mutex<Option<gix::status::Outcome>>>,
}

#[pymethods]
impl StatusOutcome {
    fn has_changes(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("status outcome is in use"))?
            .as_ref()
            .is_some_and(|v| v.has_changes()))
    }
    fn write_changes(&self, py: Python<'_>) -> PyResult<bool> {
        let inner = self.inner.clone();
        runtime::run(py, "write status changes", None, None, move |_| {
            let mut inner = inner
                .try_lock()
                .map_err(|_| PyRuntimeError::new_err("status outcome is in use"))?;
            inner
                .as_mut()
                .map(|v| v.write_changes())
                .flatten()
                .transpose()
                .map(|v| v.is_some())
                .map_err(to_py)
        })?
    }
    #[getter]
    fn worktree_index(&self) -> PyResult<Option<IndexFile>> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("status outcome is in use"))?;
        Ok(inner.as_ref().map(|v| match &v.worktree_index {
            gix::worktree::IndexPersistedOrInMemory::Persisted(index) => IndexFile::from_shared(index.clone()),
            gix::worktree::IndexPersistedOrInMemory::InMemory(index) => IndexFile::from_native(index.clone()),
        }))
    }
}

#[pymethods]
impl Repository {
    #[pyo3(signature = (progress=None, *, cancel=None))]
    fn status(&self, progress: Option<Progress>, cancel: Option<CancellationToken>) -> StatusPlatform {
        StatusPlatform {
            handle: self.handle.clone(),
            progress,
            cancel,
            untracked: None,
            index: None,
            head: None,
            rewrites: None,
            tree_renames: Default::default(),
            submodules: None,
            thread_limit: None,
            sorting: false,
            ignored: false,
        }
    }
    fn is_dirty(&self, py: Python<'_>) -> PyResult<bool> {
        self.handle.run(py, |repo| repo.is_dirty().map_err(to_py))
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<StatusPlatform>()?;
    m.add_class::<StatusIter>()?;
    m.add_class::<StatusItem>()?;
    m.add_class::<StatusOutcome>()?;
    m.add_class::<SubmoduleStatus>()?;
    Ok(())
}
