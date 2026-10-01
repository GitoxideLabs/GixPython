use crate::{
    diff_options::{Rewrites, algorithm},
    error::to_py,
    objects::Time,
    repository::Repository,
    runtime::{self, CancellationToken, OwnedIter, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};
use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::sync::Arc;

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct BlameOptions {
    inner: gix::repository::blame_file::Options,
}
#[pymethods]
impl BlameOptions {
    #[new]
    #[pyo3(signature=(*,diff_algorithm=None,ranges=None,since=None,rewrites=None))]
    fn new(
        diff_algorithm: Option<&str>,
        ranges: Option<Vec<(u32, u32)>>,
        since: Option<Time>,
        rewrites: Option<Rewrites>,
    ) -> PyResult<Self> {
        let ranges = ranges.unwrap_or_default();
        if ranges.iter().any(|(start, end)| *start == 0 || end < start) {
            return Err(PyValueError::new_err(
                "blame ranges are ordered, one-based inclusive intervals",
            ));
        }
        Ok(Self {
            inner: gix::repository::blame_file::Options {
                diff_algorithm: diff_algorithm.map(algorithm).transpose()?,
                ranges: gix::blame::BlameRanges::from_one_based_inclusive_ranges(
                    ranges.into_iter().map(|(start, end)| start..=end).collect(),
                )
                .map_err(to_py)?,
                since: since.map(|v| gix::date::Time {
                    seconds: v.seconds,
                    offset: v.offset,
                }),
                rewrites: rewrites.map(|v| v.inner),
            },
        })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct BlameEntry {
    inner: gix::blame::BlameEntry,
}
#[pymethods]
impl BlameEntry {
    #[getter]
    fn start_in_blamed_file(&self) -> u32 {
        self.inner.start_in_blamed_file
    }
    #[getter]
    fn start_in_source_file(&self) -> u32 {
        self.inner.start_in_source_file
    }
    #[getter]
    fn len(&self) -> u32 {
        self.inner.len.get()
    }
    #[getter]
    fn commit_id(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.commit_id,
        }
    }
    #[getter]
    fn source_file_name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner.source_file_name.as_ref().map(|v| PyBytes::new(py, v))
    }
    fn range_in_blamed_file(&self) -> (usize, usize) {
        let r = self.inner.range_in_blamed_file();
        (r.start, r.end)
    }
    fn range_in_source_file(&self) -> (usize, usize) {
        let r = self.inner.range_in_source_file();
        (r.start, r.end)
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct BlameOutcome {
    inner: Arc<gix::blame::Outcome>,
}
#[pymethods]
impl BlameOutcome {
    #[getter]
    fn entries(&self) -> Vec<BlameEntry> {
        self.inner
            .entries
            .iter()
            .cloned()
            .map(|inner| BlameEntry { inner })
            .collect()
    }
    #[getter]
    fn blob<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.blob)
    }
    #[getter]
    fn statistics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = &self.inner.statistics;
        let out = PyDict::new(py);
        for (key, value) in [
            ("commits_traversed", s.commits_traversed),
            ("trees_decoded", s.trees_decoded),
            ("trees_diffed", s.trees_diffed),
            ("trees_diffed_with_rewrites", s.trees_diffed_with_rewrites),
            ("blobs_diffed", s.blobs_diffed),
        ] {
            out.set_item(key, value)?;
        }
        Ok(out)
    }
    fn entries_with_lines(&self) -> BlameLines {
        let outcome = self.inner.clone();
        BlameLines {
            inner: OwnedIter::new("blame lines", None, None, move |_, producer| {
                producer.serve(
                    outcome
                        .entries_with_lines()
                        .map(|(entry, lines)| Ok((BlameEntry { inner: entry }, lines))),
                )
            }),
        }
    }
}
#[pyclass(frozen, module = "gix")]
pub struct BlameLines {
    inner: OwnedIter<(BlameEntry, Vec<gix::bstr::BString>), PyErr>,
}
#[pymethods]
impl BlameLines {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Option<(BlameEntry, Vec<Bound<'py, PyBytes>>)>> {
        Ok(self
            .inner
            .next(py)?
            .transpose()?
            .map(|(entry, lines)| (entry, lines.iter().map(|v| PyBytes::new(py, v)).collect())))
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
        _v: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}
#[pymethods]
impl Repository {
    #[pyo3(signature=(file_path,suspect,options=None,*,progress=None,cancel=None))]
    fn blame_file(
        &self,
        py: Python<'_>,
        file_path: &Bound<'_, PyAny>,
        suspect: &Bound<'_, PyAny>,
        options: Option<BlameOptions>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<BlameOutcome> {
        let path = bytes(file_path)?;
        let suspect = ObjectSpec::extract(suspect)?;
        let handle = self.handle.clone();
        runtime::run(py, "blame", progress, cancel, move |_| {
            handle.with(|repo| {
                repo.blame_file(
                    path.as_bstr(),
                    suspect.resolve(repo)?,
                    options.unwrap_or_default().inner,
                )
                .map(|inner| BlameOutcome { inner: Arc::new(inner) })
                .map_err(to_py)
            })
        })?
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<BlameOptions>()?;
    m.add_class::<BlameEntry>()?;
    m.add_class::<BlameOutcome>()?;
    m.add_class::<BlameLines>()?;
    Ok(())
}
