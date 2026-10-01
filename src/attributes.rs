//! Native attribute and exclude stacks, with reusable outcomes and lazy result iteration.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
};

use crate::{
    error::to_py,
    index::IndexFile,
    pathspec::attributes_source,
    repository::{RepoHandle, Repository},
    runtime::OwnedIter,
    types::bytes,
};

fn busy() -> PyErr {
    PyRuntimeError::new_err("attribute state is already in use")
}

fn ignore_source(value: &str) -> PyResult<gix::worktree::stack::state::ignore::Source> {
    use gix::worktree::stack::state::ignore::Source;
    match value {
        "id_mapping" => Ok(Source::IdMapping),
        "worktree_then_id_mapping_if_not_skipped" => Ok(Source::WorktreeThenIdMappingIfNotSkipped),
        _ => Err(PyValueError::new_err(
            "ignore source must be id_mapping or worktree_then_id_mapping_if_not_skipped",
        )),
    }
}

#[pyclass(frozen, module = "gix")]
pub struct IgnoreSearch {
    pub(crate) inner: gix::ignore::Search,
}

#[pymethods]
impl IgnoreSearch {
    #[staticmethod]
    #[pyo3(signature = (patterns, *, support_precious=false))]
    fn from_overrides(patterns: Vec<Bound<'_, PyAny>>, support_precious: bool) -> PyResult<Self> {
        let patterns = patterns
            .iter()
            .map(|value| bytes(value).map(|v| gix::path::from_bstr(v.as_bstr()).into_owned().into_os_string()))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            inner: gix::ignore::Search::from_overrides(patterns, gix::ignore::search::Ignore { support_precious }),
        })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct AttributeStack {
    handle: RepoHandle,
    inner: Arc<Mutex<gix::worktree::Stack>>,
    attributes: bool,
    excludes: bool,
}

impl AttributeStack {
    pub fn from_native(handle: RepoHandle, inner: gix::worktree::Stack) -> Self {
        use gix::worktree::stack::State;
        let attributes = !matches!(inner.state(), State::IgnoreStack(_));
        let excludes = matches!(
            inner.state(),
            State::IgnoreStack(_) | State::AttributesAndIgnoreStack { .. }
        );
        Self {
            handle,
            inner: Arc::new(Mutex::new(inner)),
            attributes,
            excludes,
        }
    }
    fn require_attributes(&self) -> PyResult<()> {
        if self.attributes {
            Ok(())
        } else {
            Err(PyValueError::new_err("this stack was configured without attributes"))
        }
    }
    fn require_excludes(&self) -> PyResult<()> {
        if self.excludes {
            Ok(())
        } else {
            Err(PyValueError::new_err("this stack was configured without excludes"))
        }
    }
    fn platform(&self, py: Python<'_>, relative: PathBuf, mode: Option<u32>) -> PyResult<AttributePlatform> {
        let mode = mode
            .map(|v| gix::index::entry::Mode::from_bits(v).ok_or_else(|| PyValueError::new_err("invalid index mode")))
            .transpose()?;
        let inner = self.inner.clone();
        let native_path = relative.clone();
        let path = self.handle.run(py, move |repo| {
            let mut stack = inner.try_lock().map_err(|_| busy())?;
            stack
                .at_path(native_path.as_path(), mode, &repo.objects)
                .map(|platform| platform.path().to_owned())
                .map_err(to_py)
        })?;
        Ok(AttributePlatform {
            stack: self.clone(),
            relative,
            mode,
            path,
        })
    }
}

fn statistics<'py>(py: Python<'py>, stats: gix::worktree::stack::Statistics) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("platforms", stats.platforms)?;
    out.set_item("num_mkdir_calls", stats.delegate.num_mkdir_calls)?;
    out.set_item("push_element", stats.delegate.push_element)?;
    out.set_item("push_directory", stats.delegate.push_directory)?;
    out.set_item("pop_directory", stats.delegate.pop_directory)?;
    let attributes = PyDict::new(py);
    attributes.set_item("patterns_buffers", stats.attributes.patterns_buffers)?;
    attributes.set_item("pattern_files", stats.attributes.pattern_files)?;
    attributes.set_item("tried_pattern_files", stats.attributes.tried_pattern_files)?;
    out.set_item("attributes", attributes)?;
    let ignore = PyDict::new(py);
    ignore.set_item("patterns_buffers", stats.ignore.patterns_buffers)?;
    ignore.set_item("pattern_files", stats.ignore.pattern_files)?;
    ignore.set_item("tried_pattern_files", stats.ignore.tried_pattern_files)?;
    out.set_item("ignore", ignore)?;
    Ok(out)
}

#[pymethods]
impl AttributeStack {
    fn base(&self) -> PyResult<PathBuf> {
        Ok(self.inner.try_lock().map_err(|_| busy())?.base().to_owned())
    }
    #[pyo3(signature = (relative, mode=None))]
    fn at_path(&self, py: Python<'_>, relative: PathBuf, mode: Option<u32>) -> PyResult<AttributePlatform> {
        self.platform(py, relative, mode)
    }
    #[pyo3(signature = (relative, mode=None))]
    fn at_entry(&self, py: Python<'_>, relative: &Bound<'_, PyAny>, mode: Option<u32>) -> PyResult<AttributePlatform> {
        self.platform(py, gix::path::from_bstr(bytes(relative)?.as_bstr()).into_owned(), mode)
    }
    fn attribute_matches(&self) -> PyResult<AttributeOutcome> {
        self.require_attributes()?;
        Ok(AttributeOutcome {
            inner: Arc::new(Mutex::new(
                self.inner.try_lock().map_err(|_| busy())?.attribute_matches(),
            )),
        })
    }
    fn selected_attribute_matches(&self, given: Vec<String>) -> PyResult<AttributeOutcome> {
        self.require_attributes()?;
        Ok(AttributeOutcome {
            inner: Arc::new(Mutex::new(
                self.inner
                    .try_lock()
                    .map_err(|_| busy())?
                    .selected_attribute_matches(given.iter().map(String::as_str)),
            )),
        })
    }
    fn statistics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stats = *self.inner.try_lock().map_err(|_| busy())?.statistics();
        statistics(py, stats)
    }
    fn take_statistics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stats = self.inner.try_lock().map_err(|_| busy())?.take_statistics();
        statistics(py, stats)
    }
}

#[pyclass(frozen, module = "gix")]
pub struct AttributePlatform {
    stack: AttributeStack,
    relative: PathBuf,
    mode: Option<gix::index::entry::Mode>,
    path: PathBuf,
}

#[pyclass(frozen, module = "gix")]
pub struct ExcludeMatch {
    pattern: gix::glob::Pattern,
    #[pyo3(get)]
    source: Option<PathBuf>,
    #[pyo3(get)]
    sequence_number: usize,
    #[pyo3(get)]
    kind: String,
}

#[pymethods]
impl ExcludeMatch {
    #[getter]
    fn pattern<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.pattern.text)
    }
    fn is_negative(&self) -> bool {
        self.pattern.is_negative()
    }
}

#[pymethods]
impl AttributePlatform {
    fn path(&self) -> PathBuf {
        self.path.clone()
    }
    fn matching_exclude_pattern(&self, py: Python<'_>) -> PyResult<Option<ExcludeMatch>> {
        self.stack.require_excludes()?;
        let stack = self.stack.inner.clone();
        let relative = self.relative.clone();
        let mode = self.mode;
        self.stack.handle.run(py, move |repo| {
            let mut stack = stack.try_lock().map_err(|_| busy())?;
            let platform = stack.at_path(relative.as_path(), mode, &repo.objects).map_err(to_py)?;
            Ok(platform.matching_exclude_pattern().map(|value| ExcludeMatch {
                pattern: value.pattern.clone(),
                source: value.source.map(ToOwned::to_owned),
                sequence_number: value.sequence_number,
                kind: format!("{:?}", value.kind),
            }))
        })
    }
    fn is_excluded(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(self
            .matching_exclude_pattern(py)?
            .is_some_and(|value| !value.pattern.is_negative()))
    }
    fn excluded_kind(&self, py: Python<'_>) -> PyResult<Option<String>> {
        Ok(self
            .matching_exclude_pattern(py)?
            .filter(|value| !value.pattern.is_negative())
            .map(|value| value.kind))
    }
    fn matching_attributes(&self, py: Python<'_>, out: &AttributeOutcome) -> PyResult<bool> {
        self.stack.require_attributes()?;
        let stack = self.stack.inner.clone();
        let relative = self.relative.clone();
        let mode = self.mode;
        let output = out.inner.clone();
        self.stack.handle.run(py, move |repo| {
            let mut stack = stack.try_lock().map_err(|_| busy())?;
            let mut output = output.try_lock().map_err(|_| busy())?;
            let platform = stack.at_path(relative.as_path(), mode, &repo.objects).map_err(to_py)?;
            Ok(platform.matching_attributes(&mut output))
        })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct AttributeMatch {
    pattern: Vec<u8>,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    state: &'static str,
    value: Option<Vec<u8>>,
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    source_id: Option<usize>,
    #[pyo3(get)]
    source: Option<PathBuf>,
    #[pyo3(get)]
    sequence_number: usize,
}

impl AttributeMatch {
    fn from_native(value: gix::attrs::search::Match<'_>) -> Self {
        let (state, assignment) = match value.assignment.state {
            gix::attrs::StateRef::Set => ("set", None),
            gix::attrs::StateRef::Unset => ("unset", None),
            gix::attrs::StateRef::Unspecified => ("unspecified", None),
            gix::attrs::StateRef::Value(value) => ("value", Some(value.as_bstr().to_vec())),
        };
        Self {
            pattern: value.pattern.text.to_vec(),
            name: value.assignment.name.as_ref().to_owned(),
            state,
            value: assignment,
            kind: if matches!(value.kind, gix::attrs::search::MatchKind::Macro { .. }) {
                "macro"
            } else {
                "attribute"
            },
            source_id: value.kind.source_id().map(|id| id.0),
            source: value.location.source.map(ToOwned::to_owned),
            sequence_number: value.location.sequence_number,
        }
    }
}

#[pymethods]
impl AttributeMatch {
    #[getter]
    fn pattern<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.pattern)
    }
    #[getter]
    fn value<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.value.as_ref().map(|value| PyBytes::new(py, value))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct AttributeOutcome {
    inner: Arc<Mutex<gix::attrs::search::Outcome>>,
}

impl AttributeOutcome {
    fn iterator(&self, selected: bool) -> PyResult<AttributeMatches> {
        let outcome = self.inner.try_lock().map_err(|_| busy())?.clone();
        Ok(AttributeMatches {
            inner: OwnedIter::new("attribute matches", None, None, move |_, producer| {
                if selected {
                    producer.serve(
                        outcome
                            .iter_selected()
                            .map(|value| Ok(AttributeMatch::from_native(value))),
                    )
                } else {
                    producer.serve(outcome.iter().map(|value| Ok(AttributeMatch::from_native(value))))
                }
            }),
        })
    }
}

#[pymethods]
impl AttributeOutcome {
    fn iter(&self) -> PyResult<AttributeMatches> {
        self.iterator(false)
    }
    fn iter_selected(&self) -> PyResult<AttributeMatches> {
        self.iterator(true)
    }
    fn is_done(&self) -> PyResult<bool> {
        Ok(self.inner.try_lock().map_err(|_| busy())?.is_done())
    }
    fn reset(&self) -> PyResult<()> {
        self.inner.try_lock().map_err(|_| busy())?.reset();
        Ok(())
    }
    fn match_by_id(&self, id: usize) -> PyResult<Option<AttributeMatch>> {
        Ok(self
            .inner
            .try_lock()
            .map_err(|_| busy())?
            .match_by_id(gix::attrs::search::AttributeId(id))
            .map(AttributeMatch::from_native))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct AttributeMatches {
    inner: OwnedIter<AttributeMatch, PyErr>,
}

#[pymethods]
impl AttributeMatches {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<AttributeMatch>> {
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
impl Repository {
    #[pyo3(signature = (index, attributes_source="worktree_then_id_mapping", ignore_source="worktree_then_id_mapping_if_not_skipped", exclude_overrides=None))]
    fn attributes(
        &self,
        py: Python<'_>,
        index: &IndexFile,
        attributes_source: &str,
        ignore_source: &str,
        exclude_overrides: Option<&IgnoreSearch>,
    ) -> PyResult<AttributeStack> {
        let index = index.snapshot()?;
        let attributes = self::attributes_source(attributes_source)?;
        let ignore = self::ignore_source(ignore_source)?;
        let overrides = exclude_overrides.map(|value| value.inner.clone());
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            repo.attributes(&index, attributes, ignore, overrides)
                .map(|stack| AttributeStack::from_native(handle, stack.detach()))
                .map_err(to_py)
        })
    }
    #[pyo3(signature = (index, attributes_source="worktree_then_id_mapping"))]
    fn attributes_only(&self, py: Python<'_>, index: &IndexFile, attributes_source: &str) -> PyResult<AttributeStack> {
        let index = index.snapshot()?;
        let source = self::attributes_source(attributes_source)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            repo.attributes_only(&index, source)
                .map(|stack| AttributeStack::from_native(handle, stack.detach()))
                .map_err(to_py)
        })
    }
    #[pyo3(signature = (index, overrides=None, source="worktree_then_id_mapping_if_not_skipped"))]
    fn excludes(
        &self,
        py: Python<'_>,
        index: &IndexFile,
        overrides: Option<&IgnoreSearch>,
        source: &str,
    ) -> PyResult<AttributeStack> {
        let index = index.snapshot()?;
        let source = ignore_source(source)?;
        let overrides = overrides.map(|value| value.inner.clone());
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            repo.excludes(&index, overrides, source)
                .map(|stack| AttributeStack::from_native(handle, stack.detach()))
                .map_err(to_py)
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<IgnoreSearch>()?;
    m.add_class::<AttributeStack>()?;
    m.add_class::<AttributePlatform>()?;
    m.add_class::<ExcludeMatch>()?;
    m.add_class::<AttributeMatch>()?;
    m.add_class::<AttributeOutcome>()?;
    m.add_class::<AttributeMatches>()?;
    Ok(())
}
