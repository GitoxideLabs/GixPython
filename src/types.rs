use std::hash::{Hash, Hasher};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyString},
};

#[cfg(not(feature = "revision"))]
use crate::error::FeatureUnavailableError;
#[cfg(feature = "revision")]
use crate::error::to_py;

/// Lossless Git bytes; Python text is explicitly encoded as UTF-8.
pub fn bytes(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(value) = value.cast::<PyBytes>() {
        Ok(value.as_bytes().to_vec())
    } else if let Ok(value) = value.cast::<PyString>() {
        Ok(value.to_str()?.as_bytes().to_vec())
    } else {
        Err(PyTypeError::new_err("expected str or bytes"))
    }
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct HashKind {
    pub inner: gix::hash::Kind,
}

#[pymethods]
impl HashKind {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        name.parse()
            .map(|inner| Self { inner })
            .map_err(|_| PyValueError::new_err("hash kind is unknown or disabled in this build"))
    }
    #[classattr]
    #[pyo3(name = "SHA1")]
    fn sha1() -> Option<Self> {
        "sha1".parse().ok().map(|inner| Self { inner })
    }
    #[classattr]
    #[pyo3(name = "SHA256")]
    fn sha256() -> Option<Self> {
        "sha256".parse().ok().map(|inner| Self { inner })
    }
    fn len_in_bytes(&self) -> usize {
        self.inner.len_in_bytes()
    }
    fn len_in_hex(&self) -> usize {
        self.inner.len_in_hex()
    }
    fn null(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.null(),
        }
    }
    fn empty_blob(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.empty_blob(),
        }
    }
    fn empty_tree(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.empty_tree(),
        }
    }
    #[staticmethod]
    fn all() -> Vec<Self> {
        gix::hash::Kind::all()
            .iter()
            .map(|inner| Self { inner: *inner })
            .collect()
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self) -> String {
        format!("HashKind('{}')", self.inner)
    }
    fn __hash__(&self) -> usize {
        self.inner.len_in_bytes()
    }
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ObjectId {
    pub inner: gix::ObjectId,
}

#[pymethods]
impl ObjectId {
    #[new]
    fn new(hex: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::from_hex(hex)
    }
    #[staticmethod]
    fn from_hex(hex: &Bound<'_, PyAny>) -> PyResult<Self> {
        gix::ObjectId::from_hex(&bytes(hex)?)
            .map(|inner| Self { inner })
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
    fn kind(&self) -> HashKind {
        HashKind {
            inner: self.inner.kind(),
        }
    }
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }
    fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.as_bytes())
    }
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        self.as_bytes(py)
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self) -> String {
        format!("ObjectId('{}')", self.inner)
    }
    fn __hash__(&self) -> u64 {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut state);
        state.finish()
    }
}

#[derive(Clone)]
pub enum ObjectSpec {
    Id(gix::ObjectId),
    Revision(Vec<u8>),
}

impl ObjectSpec {
    pub fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(id) = value.extract::<ObjectId>() {
            return Ok(Self::Id(id.inner));
        }
        if let Some(id) = crate::objects::object_id(value) {
            return Ok(Self::Id(id));
        }
        bytes(value).map(Self::Revision)
    }
    pub fn resolve(&self, repo: &gix::Repository) -> PyResult<gix::ObjectId> {
        let id = match self {
            Self::Id(id) => *id,
            Self::Revision(spec) => {
                #[cfg(feature = "revision")]
                {
                    repo.rev_parse_single(spec.as_bstr()).map_err(to_py)?.detach()
                }
                #[cfg(not(feature = "revision"))]
                {
                    gix::ObjectId::from_hex(spec).map_err(|_| {
                        FeatureUnavailableError::new_err("revision support is disabled; pass a full object ID")
                    })?
                }
            }
        };
        if id.kind() != repo.object_hash() {
            return Err(PyValueError::new_err("object ID hash kind differs from the repository"));
        }
        Ok(id)
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ObjectId>()?;
    m.add_class::<HashKind>()?;
    Ok(())
}
