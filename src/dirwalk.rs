//! Directory walks retain their native producer and outcome without eager collection.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};

use crate::{
    error::to_py,
    index::{IndexFile, IndexSnapshot},
    pathspec::Pathspec,
    repository::{RepoHandle, Repository},
    runtime::{CancellationToken, OwnedIter, Progress},
    types::bytes,
};

fn emission(value: &str) -> PyResult<gix::dir::walk::EmissionMode> {
    match value {
        "matching" => Ok(gix::dir::walk::EmissionMode::Matching),
        "collapse_directory" => Ok(gix::dir::walk::EmissionMode::CollapseDirectory),
        _ => Err(PyValueError::new_err("emission must be matching or collapse_directory")),
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Copy)]
pub struct DirwalkOptions {
    pub inner: gix::dirwalk::Options,
}

#[pymethods]
impl DirwalkOptions {
    fn empty_patterns_match_prefix(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.empty_patterns_match_prefix(toggle),
        }
    }
    fn recurse_repositories(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.recurse_repositories(toggle),
        }
    }
    fn emit_pruned(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.emit_pruned(toggle),
        }
    }
    fn emit_ignored(&self, value: Option<&str>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.emit_ignored(value.map(emission).transpose()?),
        })
    }
    fn emit_untracked(&self, value: &str) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.emit_untracked(emission(value)?),
        })
    }
    fn emit_tracked(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.emit_tracked(toggle),
        }
    }
    fn emit_empty_directories(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.emit_empty_directories(toggle),
        }
    }
    fn classify_untracked_bare_repositories(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.classify_untracked_bare_repositories(toggle),
        }
    }
    fn symlinks_to_directories_are_ignored_like_directories(&self, toggle: bool) -> Self {
        Self {
            inner: self.inner.symlinks_to_directories_are_ignored_like_directories(toggle),
        }
    }
    fn emit_collapsed(&self, value: Option<&str>) -> PyResult<Self> {
        use gix::dir::walk::CollapsedEntriesEmissionMode;
        let value = match value {
            None => None,
            Some("on_status_mismatch") => Some(CollapsedEntriesEmissionMode::OnStatusMismatch),
            Some("all") => Some(CollapsedEntriesEmissionMode::All),
            _ => {
                return Err(PyValueError::new_err(
                    "collapsed emission must be on_status_mismatch, all, or None",
                ));
            }
        };
        Ok(Self {
            inner: self.inner.emit_collapsed(value),
        })
    }
    fn for_deletion(&self, value: Option<&str>) -> PyResult<Self> {
        use gix::dir::walk::ForDeletionMode;
        let value = match value {
            None => None,
            Some("ignored_directories_can_hide_nested_repositories") => {
                Some(ForDeletionMode::IgnoredDirectoriesCanHideNestedRepositories)
            }
            Some("find_non_bare_repositories_in_ignored_directories") => {
                Some(ForDeletionMode::FindNonBareRepositoriesInIgnoredDirectories)
            }
            Some("find_repositories_in_ignored_directories") => {
                Some(ForDeletionMode::FindRepositoriesInIgnoredDirectories)
            }
            _ => return Err(PyValueError::new_err("unknown deletion classification mode")),
        };
        Ok(Self {
            inner: self.inner.for_deletion(value),
        })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct DirwalkEntry {
    inner: gix::dir::Entry,
}

#[pymethods]
impl DirwalkEntry {
    #[getter]
    fn rela_path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.rela_path)
    }
    #[getter]
    fn status(&self) -> String {
        format!("{:?}", self.inner.status)
    }
    #[getter]
    fn property(&self) -> Option<String> {
        self.inner.property.map(|v| format!("{v:?}"))
    }
    #[getter]
    fn disk_kind(&self) -> Option<String> {
        self.inner.disk_kind.map(|v| format!("{v:?}"))
    }
    #[getter]
    fn index_kind(&self) -> Option<String> {
        self.inner.index_kind.map(|v| format!("{v:?}"))
    }
    #[getter]
    fn pathspec_match(&self) -> Option<String> {
        self.inner.pathspec_match.map(|v| format!("{v:?}"))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct DirwalkItem {
    inner: gix::dirwalk::iter::Item,
}

#[pymethods]
impl DirwalkItem {
    #[getter]
    fn entry(&self) -> DirwalkEntry {
        DirwalkEntry {
            inner: self.inner.entry.clone(),
        }
    }
    #[getter]
    fn collapsed_directory_status(&self) -> Option<String> {
        self.inner.collapsed_directory_status.map(|v| format!("{v:?}"))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct DirwalkStatistics {
    #[pyo3(get)]
    read_dir_calls: u32,
    #[pyo3(get)]
    returned_entries: usize,
    #[pyo3(get)]
    seen_entries: u32,
}

#[pyclass(frozen, module = "gix")]
pub struct DirwalkOutcome {
    pub handle: RepoHandle,
    pub inner: Mutex<gix::dirwalk::iter::Outcome>,
}

#[pymethods]
impl DirwalkOutcome {
    #[getter]
    fn excludes(&self) -> PyResult<crate::attributes::AttributeStack> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("dirwalk outcome is already in use"))?;
        Ok(crate::attributes::AttributeStack::from_native(
            self.handle.clone(),
            inner.excludes.clone(),
        ))
    }
    #[getter]
    fn index(&self) -> PyResult<IndexFile> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("dirwalk outcome is already in use"))?;
        Ok(match &inner.index {
            gix::worktree::IndexPersistedOrInMemory::Persisted(index) => IndexFile::from_shared(index.clone()),
            gix::worktree::IndexPersistedOrInMemory::InMemory(index) => IndexFile::from_native(index.clone()),
        })
    }
    #[getter]
    fn pathspec(&self) -> PyResult<Pathspec> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("dirwalk outcome is already in use"))?;
        Ok(Pathspec::from_native(inner.pathspec.clone()))
    }
    #[getter]
    fn traversal_root(&self) -> PyResult<PathBuf> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("dirwalk outcome is already in use"))?;
        Ok(inner.traversal_root.clone())
    }
    #[getter]
    fn dirwalk(&self) -> PyResult<DirwalkStatistics> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("dirwalk outcome is already in use"))?;
        Ok(DirwalkStatistics {
            read_dir_calls: inner.dirwalk.read_dir_calls,
            returned_entries: inner.dirwalk.returned_entries,
            seen_entries: inner.dirwalk.seen_entries,
        })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct DirwalkIter {
    handle: RepoHandle,
    inner: OwnedIter<DirwalkItem, PyErr>,
    outcome: Arc<Mutex<Option<gix::dirwalk::iter::Outcome>>>,
}

#[pymethods]
impl DirwalkIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<DirwalkItem>> {
        self.inner.next(py)?.transpose()
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.inner.close(py)
    }
    #[allow(
        clippy::wrong_self_convention,
        reason = "Preserves the native gix method name; the outcome is consumed through interior mutability"
    )]
    fn into_outcome(&self, py: Python<'_>) -> PyResult<Option<DirwalkOutcome>> {
        self.inner.close(py)?;
        Ok(self
            .outcome
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("dirwalk outcome is already in use"))?
            .take()
            .map(|inner| DirwalkOutcome {
                handle: self.handle.clone(),
                inner: Mutex::new(inner),
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

#[pymethods]
impl Repository {
    fn dirwalk_options(&self, py: Python<'_>) -> PyResult<DirwalkOptions> {
        self.handle.run(py, |repo| {
            repo.dirwalk_options()
                .map(|inner| DirwalkOptions { inner })
                .map_err(to_py)
        })
    }
    #[pyo3(signature = (index, patterns, options, *, progress=None, cancel=None))]
    fn dirwalk_iter(
        &self,
        index: &IndexFile,
        patterns: Vec<Bound<'_, PyAny>>,
        options: DirwalkOptions,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<DirwalkIter> {
        let index = index.snapshot()?;
        let patterns = patterns
            .iter()
            .map(|value| bytes(value).map(gix::bstr::BString::from))
            .collect::<PyResult<Vec<_>>>()?;
        let handle = self.handle.clone();
        let outcome = Arc::new(Mutex::new(None));
        let output = outcome.clone();
        let inner = OwnedIter::new("directory walk", progress, cancel, move |context, producer| {
            handle.with(|repo| {
                let index = match index {
                    IndexSnapshot::Shared(index) => gix::worktree::IndexPersistedOrInMemory::Persisted(index),
                    IndexSnapshot::Owned(index) => {
                        gix::worktree::IndexPersistedOrInMemory::InMemory(Arc::unwrap_or_clone(index))
                    }
                };
                let mut iter = repo
                    .dirwalk_iter(index, patterns, context.interrupt.into(), options.inner)
                    .map_err(to_py)?;
                producer.serve(
                    iter.by_ref()
                        .map(|value| value.map(|inner| DirwalkItem { inner }).map_err(to_py)),
                )?;
                *output.lock().unwrap_or_else(|e| e.into_inner()) = iter.into_outcome();
                Ok(())
            })
        });
        Ok(DirwalkIter {
            handle: self.handle.clone(),
            inner,
            outcome,
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DirwalkOptions>()?;
    m.add_class::<DirwalkEntry>()?;
    m.add_class::<DirwalkItem>()?;
    m.add_class::<DirwalkIter>()?;
    m.add_class::<DirwalkOutcome>()?;
    m.add_class::<DirwalkStatistics>()?;
    Ok(())
}
