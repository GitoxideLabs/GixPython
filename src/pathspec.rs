//! Native pathspec matching with byte-preserving patterns and lazy index selection.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};

use crate::{
    error::to_py,
    index::{IndexEntry, IndexFile},
    repository::Repository,
    runtime::{self, CancellationToken, OwnedIter, Progress},
    types::bytes,
};

pub fn attributes_source(value: &str) -> PyResult<gix::worktree::stack::state::attributes::Source> {
    use gix::worktree::stack::state::attributes::Source;
    match value {
        "id_mapping" => Ok(Source::IdMapping),
        "id_mapping_then_worktree" => Ok(Source::IdMappingThenWorktree),
        "worktree_then_id_mapping" => Ok(Source::WorktreeThenIdMapping),
        _ => Err(PyValueError::new_err(
            "attribute source must be id_mapping, id_mapping_then_worktree, or worktree_then_id_mapping",
        )),
    }
}

#[pyclass(frozen, module = "gix")]
pub struct Pathspec {
    inner: Arc<Mutex<gix::PathspecDetached>>,
}

impl Pathspec {
    pub fn from_native(inner: gix::PathspecDetached) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
    pub fn snapshot(&self) -> PyResult<gix::PathspecDetached> {
        self.inner
            .try_lock()
            .map(|v| v.clone())
            .map_err(|_| PyRuntimeError::new_err("pathspec is already in use"))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct PathspecPattern {
    inner: gix::pathspec::Pattern,
}

#[pymethods]
impl PathspecPattern {
    fn is_nil(&self) -> bool {
        self.inner.is_nil()
    }
    fn is_excluded(&self) -> bool {
        self.inner.is_excluded()
    }
    fn always_matches(&self) -> bool {
        self.inner.always_matches()
    }
    fn path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.path())
    }
    fn prefix_directory<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.prefix_directory())
    }
    fn to_bstring<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.to_bstring())
    }
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        self.to_bstring(py)
    }
}

#[pyclass(frozen, module = "gix")]
pub struct PathspecMatch {
    pattern: gix::pathspec::Pattern,
    #[pyo3(get)]
    sequence_number: usize,
    #[pyo3(get)]
    kind: String,
}

impl PathspecMatch {
    fn from_native(value: gix::pathspec::search::Match<'_>) -> Self {
        Self {
            pattern: value.pattern.clone(),
            sequence_number: value.sequence_number,
            kind: format!("{:?}", value.kind),
        }
    }
}

#[pymethods]
impl PathspecMatch {
    #[getter]
    fn pattern(&self) -> PathspecPattern {
        PathspecPattern {
            inner: self.pattern.clone(),
        }
    }
    fn is_excluded(&self) -> bool {
        self.pattern.is_excluded()
    }
}

#[pyclass(frozen, module = "gix")]
pub struct PathspecPatterns {
    inner: OwnedIter<PathspecPattern, PyErr>,
}
#[pyclass(frozen, module = "gix")]
pub struct PathspecEntries {
    inner: OwnedIter<IndexEntry, PyErr>,
}

macro_rules! iterator {
    ($name:ident, $item:ty) => {
        #[pymethods]
        impl $name {
            fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                slf
            }
            fn __next__(&self, py: Python<'_>) -> PyResult<Option<$item>> {
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
    };
}
iterator!(PathspecPatterns, PathspecPattern);

#[pymethods]
impl PathspecEntries {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<(Py<PyBytes>, IndexEntry)>> {
        Ok(self
            .inner
            .next(py)?
            .transpose()?
            .map(|entry| (PyBytes::new(py, &entry.path).unbind(), entry)))
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

#[pyclass(frozen, module = "gix")]
pub struct PathspecSearch {
    inner: gix::pathspec::Search,
}

#[pymethods]
impl PathspecSearch {
    fn patterns(&self) -> PathspecPatterns {
        let search = self.inner.clone();
        PathspecPatterns {
            inner: OwnedIter::new("pathspec patterns", None, None, move |_, producer| {
                producer.serve(
                    search
                        .patterns()
                        .map(|pattern| Ok(PathspecPattern { inner: pattern.clone() })),
                )
            }),
        }
    }
    fn common_prefix<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.common_prefix())
    }
    fn prefix_directory(&self) -> PathBuf {
        self.inner.prefix_directory().into_owned()
    }
    fn longest_common_directory(&self) -> Option<PathBuf> {
        self.inner.longest_common_directory().map(|v| v.into_owned())
    }
}

#[pymethods]
impl Pathspec {
    fn search(&self) -> PyResult<PathspecSearch> {
        Ok(PathspecSearch {
            inner: self.snapshot()?.search,
        })
    }
    #[pyo3(signature = (relative_path, is_dir=None))]
    fn pattern_matching_relative_path(
        &self,
        py: Python<'_>,
        relative_path: &Bound<'_, PyAny>,
        is_dir: Option<bool>,
    ) -> PyResult<Option<PathspecMatch>> {
        let path = bytes(relative_path)?;
        let inner = self.inner.clone();
        runtime::run(py, "pathspec match", None, None, move |_| {
            let mut matcher = inner
                .try_lock()
                .map_err(|_| PyRuntimeError::new_err("pathspec is already in use"))?;
            Ok(matcher
                .pattern_matching_relative_path(path.as_bstr(), is_dir)
                .map(PathspecMatch::from_native))
        })?
    }
    #[pyo3(signature = (relative_path, is_dir=None))]
    fn is_included(&self, py: Python<'_>, relative_path: &Bound<'_, PyAny>, is_dir: Option<bool>) -> PyResult<bool> {
        Ok(self
            .pattern_matching_relative_path(py, relative_path, is_dir)?
            .is_some_and(|m| !m.pattern.is_excluded()))
    }
    #[pyo3(signature = (index, *, progress=None, cancel=None))]
    fn index_entries_with_paths(
        &self,
        index: &IndexFile,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<Option<PathspecEntries>> {
        let mut pathspec = self.snapshot()?;
        let index = index.snapshot()?;
        let Some(range) = index.prefixed_entries_range(pathspec.search.common_prefix()) else {
            return Ok(None);
        };
        Ok(Some(PathspecEntries {
            inner: OwnedIter::new("pathspec entries", progress, cancel, move |_, producer| {
                producer.serve(index.entries()[range].iter().filter_map(|entry| {
                    pathspec
                        .is_included(entry.path(&index), Some(false))
                        .then(|| Ok(IndexEntry::from_native(entry, &index)))
                }))
            }),
        }))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct PathspecDefaults {
    #[pyo3(get)]
    signature: u32,
    #[pyo3(get)]
    search_mode: String,
    #[pyo3(get)]
    literal: bool,
}

impl From<gix::pathspec::Defaults> for PathspecDefaults {
    fn from(value: gix::pathspec::Defaults) -> Self {
        Self {
            signature: value.signature.bits(),
            search_mode: format!("{:?}", value.search_mode),
            literal: value.literal,
        }
    }
}

#[pymethods]
impl Repository {
    #[pyo3(signature = (empty_patterns_match_prefix, patterns, inherit_ignore_case, index, attributes_source="worktree_then_id_mapping"))]
    fn pathspec(
        &self,
        py: Python<'_>,
        empty_patterns_match_prefix: bool,
        patterns: Vec<Bound<'_, PyAny>>,
        inherit_ignore_case: bool,
        index: &IndexFile,
        attributes_source: &str,
    ) -> PyResult<Pathspec> {
        let source = self::attributes_source(attributes_source)?;
        let index = index.snapshot()?;
        let patterns = patterns.iter().map(bytes).collect::<PyResult<Vec<_>>>()?;
        self.handle.run(py, move |repo| {
            repo.pathspec(
                empty_patterns_match_prefix,
                patterns.iter().map(|v| v.as_bstr()),
                inherit_ignore_case,
                &index,
                source,
            )
            .and_then(|v| v.detach())
            .map(Pathspec::from_native)
            .map_err(to_py)
        })
    }
    fn pathspec_defaults(&self, py: Python<'_>) -> PyResult<PathspecDefaults> {
        self.handle
            .run(py, |repo| repo.pathspec_defaults().map(Into::into).map_err(to_py))
    }
    fn pathspec_defaults_inherit_ignore_case(
        &self,
        py: Python<'_>,
        inherit_ignore_case: bool,
    ) -> PyResult<PathspecDefaults> {
        self.handle.run(py, move |repo| {
            repo.pathspec_defaults_inherit_ignore_case(inherit_ignore_case)
                .map(Into::into)
                .map_err(to_py)
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Pathspec>()?;
    m.add_class::<PathspecPattern>()?;
    m.add_class::<PathspecMatch>()?;
    m.add_class::<PathspecPatterns>()?;
    m.add_class::<PathspecSearch>()?;
    m.add_class::<PathspecEntries>()?;
    m.add_class::<PathspecDefaults>()?;
    Ok(())
}
