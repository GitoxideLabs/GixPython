use pyo3::{exceptions::PyValueError, prelude::*};

pub fn algorithm(name: &str) -> PyResult<gix::diff::blob::Algorithm> {
    use gix::diff::blob::Algorithm;
    match name {
        "histogram" => Ok(Algorithm::Histogram),
        "myers" => Ok(Algorithm::Myers),
        "myers_minimal" => Ok(Algorithm::MyersMinimal),
        _ => Err(PyValueError::new_err(
            "algorithm must be histogram, myers, or myers_minimal",
        )),
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Copy)]
pub struct Rewrites {
    pub inner: gix::diff::Rewrites,
}
#[pymethods]
impl Rewrites {
    #[new]
    #[pyo3(signature=(*,percentage=Some(0.5),limit=1000,track_empty=false,copies=false,copy_percentage=Some(0.5),all_sources=false))]
    fn new(
        percentage: Option<f32>,
        limit: usize,
        track_empty: bool,
        copies: bool,
        copy_percentage: Option<f32>,
        all_sources: bool,
    ) -> PyResult<Self> {
        for value in [percentage, copy_percentage].into_iter().flatten() {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(PyValueError::new_err(
                    "similarity must be a finite fraction between 0 and 1",
                ));
            }
        }
        Ok(Self {
            inner: gix::diff::Rewrites {
                percentage,
                limit,
                track_empty,
                copies: copies.then_some(gix::diff::rewrites::Copies {
                    source: if all_sources {
                        gix::diff::rewrites::CopySource::FromSetOfModifiedFilesAndAllSources
                    } else {
                        gix::diff::rewrites::CopySource::FromSetOfModifiedFiles
                    },
                    percentage: copy_percentage,
                }),
            },
        })
    }
    #[getter]
    fn percentage(&self) -> Option<f32> {
        self.inner.percentage
    }
    #[getter]
    fn limit(&self) -> usize {
        self.inner.limit
    }
    #[getter]
    fn track_empty(&self) -> bool {
        self.inner.track_empty
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Rewrites>()
}
