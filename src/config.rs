use std::{path::PathBuf, sync::Mutex};

use gix::bstr::ByteSlice;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};

use crate::{
    error::to_py,
    objects::Signature,
    repository::{MutationLease, RepoHandle, Repository},
    runtime,
    types::bytes,
};

fn closed() -> PyErr {
    PyValueError::new_err("configuration transaction is closed")
}

#[pyclass(frozen, module = "gix")]
pub struct ConfigSnapshot {
    handle: RepoHandle,
}
#[pymethods]
impl ConfigSnapshot {
    fn boolean(&self, py: Python<'_>, key: String) -> PyResult<Option<bool>> {
        self.handle
            .run(py, move |r| Ok(r.config_snapshot().boolean(key.as_str())))
    }
    fn try_boolean(&self, py: Python<'_>, key: String) -> PyResult<Option<bool>> {
        self.handle.run(py, move |r| {
            r.config_snapshot().try_boolean(key.as_str()).map_err(to_py)
        })
    }
    fn integer(&self, py: Python<'_>, key: String) -> PyResult<Option<i64>> {
        self.handle
            .run(py, move |r| Ok(r.config_snapshot().integer(key.as_str())))
    }
    fn try_integer(&self, py: Python<'_>, key: String) -> PyResult<Option<i64>> {
        self.handle.run(py, move |r| {
            r.config_snapshot().try_integer(key.as_str()).map_err(to_py)
        })
    }
    fn string<'py>(&self, py: Python<'py>, key: String) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = self
            .handle
            .run(py, move |r| Ok(r.config_snapshot().string(key.as_str())))?;
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn trusted_path(&self, py: Python<'_>, key: String) -> PyResult<Option<PathBuf>> {
        self.handle.run(py, move |r| {
            r.config_snapshot().trusted_path(key.as_str()).map_err(to_py)
        })
    }
    fn plumbing(&self, py: Python<'_>) -> PyResult<ConfigFile> {
        self.handle.run(py, |r| {
            Ok(ConfigFile::from_native(r.config_snapshot().plumbing().clone()))
        })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct ConfigFile {
    inner: Mutex<gix::config::File>,
}
impl ConfigFile {
    #[cfg(feature = "attributes")]
    pub(crate) fn snapshot(&self) -> PyResult<gix::config::File> {
        Ok(self.inner.lock().map_err(to_py)?.clone())
    }
    pub fn from_native(file: gix::config::File) -> Self {
        Self {
            inner: Mutex::new(file),
        }
    }
}
#[pymethods]
impl ConfigFile {
    fn string<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = self.inner.lock().map_err(to_py)?.string(key);
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn boolean(&self, key: &str) -> PyResult<Option<bool>> {
        self.inner.lock().map_err(to_py)?.boolean(key).map_err(to_py)
    }
    fn integer(&self, key: &str) -> PyResult<Option<i64>> {
        self.inner.lock().map_err(to_py)?.integer(key).map_err(to_py)
    }
    fn set_raw_value<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = bytes(value)?;
        let previous = self
            .inner
            .lock()
            .map_err(to_py)?
            .set_raw_value(key, value.as_bstr())
            .map_err(to_py)?;
        Ok(previous.map(|v| PyBytes::new(py, &v)))
    }
    fn to_bstring<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let data = self.inner.lock().map_err(to_py)?.to_bstring();
        Ok(PyBytes::new(py, &data))
    }
}

struct Edit {
    lease: MutationLease,
    file: gix::config::File,
}
impl Edit {
    fn apply(self) -> PyResult<Repository> {
        let handle = self.lease.handle.clone();
        self.lease.apply(|repo| {
            let mut snapshot = repo.config_snapshot_mut();
            *snapshot = self.file;
            snapshot.commit().map(|_| ()).map_err(to_py)
        })?;
        Ok(Repository { handle })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct ConfigSnapshotMut {
    inner: Mutex<Option<Edit>>,
}
#[pymethods]
impl ConfigSnapshotMut {
    #[pyo3(signature = (values, source="api"))]
    fn append_config(&self, values: &Bound<'_, PyAny>, source: &str) -> PyResult<()> {
        let values: Vec<_> = values.try_iter()?.map(|v| bytes(&v?)).collect::<PyResult<_>>()?;
        let source = source_kind(source)?;
        let mut edit = self.inner.lock().map_err(to_py)?;
        let edit = edit.as_mut().ok_or_else(closed)?;
        // Native append_config keeps source metadata and override parsing rules.
        edit.lease.apply(|repo| {
            let mut snapshot = repo.config_snapshot_mut();
            *snapshot = edit.file.clone();
            let result = snapshot
                .append_config(values.iter().map(|v| v.as_bstr()), source)
                .map(|_| ())
                .map_err(to_py);
            edit.file = snapshot.forget();
            result
        })
    }
    fn set_raw_value<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = bytes(value)?;
        let previous = {
            let mut edit = self.inner.lock().map_err(to_py)?;
            edit.as_mut()
                .ok_or_else(closed)?
                .file
                .set_raw_value(key, value.as_bstr())
                .map_err(to_py)?
        };
        Ok(previous.map(|v| PyBytes::new(py, &v)))
    }
    fn string<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = {
            let edit = self.inner.lock().map_err(to_py)?;
            edit.as_ref().ok_or_else(closed)?.file.string(key)
        };
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn commit(&self, py: Python<'_>) -> PyResult<Repository> {
        let edit = self.inner.lock().map_err(to_py)?.take().ok_or_else(closed)?;
        runtime::run(py, "configuration commit", None, None, move |_| edit.apply())?
    }
    fn forget(&self) -> PyResult<ConfigFile> {
        let edit = self.inner.lock().map_err(to_py)?.take().ok_or_else(closed)?;
        Ok(ConfigFile::from_native(edit.file))
    }
    fn commit_auto_rollback(&self, py: Python<'_>) -> PyResult<ConfigRollback> {
        let edit = self.inner.lock().map_err(to_py)?.take().ok_or_else(closed)?;
        runtime::run(py, "temporary configuration", None, None, move |_| {
            let previous = edit.lease.apply(|repo| {
                let previous = repo.config_snapshot().plumbing().clone();
                let mut snapshot = repo.config_snapshot_mut();
                *snapshot = edit.file;
                snapshot.commit().map_err(to_py)?;
                Ok(previous)
            })?;
            Ok::<_, PyErr>(ConfigRollback {
                inner: Mutex::new(Some(Edit {
                    lease: edit.lease,
                    file: previous,
                })),
            })
        })?
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
        if self.inner.lock().map_err(to_py)?.is_some() {
            self.commit(py)?;
        }
        Ok(())
    }
}
impl Drop for ConfigSnapshotMut {
    fn drop(&mut self) {
        if let Some(edit) = self.inner.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = edit.apply();
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct ConfigRollback {
    inner: Mutex<Option<Edit>>,
}
#[pymethods]
impl ConfigRollback {
    #[getter]
    fn repo(&self) -> PyResult<Repository> {
        Ok(Repository {
            handle: self
                .inner
                .lock()
                .map_err(to_py)?
                .as_ref()
                .ok_or_else(closed)?
                .lease
                .handle
                .clone(),
        })
    }
    fn rollback(&self, py: Python<'_>) -> PyResult<Repository> {
        let edit = self.inner.lock().map_err(to_py)?.take().ok_or_else(closed)?;
        runtime::run(py, "configuration rollback", None, None, move |_| edit.apply())?
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
        if self.inner.lock().map_err(to_py)?.is_some() {
            self.rollback(py)?;
        }
        Ok(())
    }
}
impl Drop for ConfigRollback {
    fn drop(&mut self) {
        if let Some(edit) = self.inner.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = edit.apply();
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct ConfigFileTransaction {
    inner: Mutex<Option<gix::config::FileTransaction>>,
}
#[pymethods]
impl ConfigFileTransaction {
    fn set_raw_value<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = bytes(value)?;
        let previous = self
            .inner
            .lock()
            .map_err(to_py)?
            .as_mut()
            .ok_or_else(closed)?
            .set_raw_value(key, value.as_bstr())
            .map_err(to_py)?;
        Ok(previous.map(|v| PyBytes::new(py, &v)))
    }
    fn string<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = self
            .inner
            .lock()
            .map_err(to_py)?
            .as_ref()
            .ok_or_else(closed)?
            .string(key);
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn commit(&self, py: Python<'_>) -> PyResult<()> {
        let transaction = self.inner.lock().map_err(to_py)?.take().ok_or_else(closed)?;
        runtime::run(py, "configuration file commit", None, None, move |_| {
            transaction.commit().map_err(to_py)
        })?
    }
    fn close(&self) -> PyResult<()> {
        self.inner.lock().map_err(to_py)?.take();
        Ok(())
    }
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __exit__(&self, _ty: &Bound<'_, PyAny>, _value: &Bound<'_, PyAny>, _tb: &Bound<'_, PyAny>) -> PyResult<()> {
        self.close()
    }
}

fn source_kind(source: &str) -> PyResult<gix::config::Source> {
    use gix::config::Source;
    match source {
        "api" => Ok(Source::Api),
        "cli" => Ok(Source::Cli),
        "local" => Ok(Source::Local),
        "user" => Ok(Source::User),
        "system" => Ok(Source::System),
        "worktree" => Ok(Source::Worktree),
        _ => Err(PyValueError::new_err("unknown configuration source")),
    }
}

#[pymethods]
impl Repository {
    fn config_snapshot(&self, py: Python<'_>) -> PyResult<ConfigSnapshot> {
        self.handle.run(py, |repo| {
            Ok(ConfigSnapshot {
                handle: RepoHandle::new(repo.clone()),
            })
        })
    }
    fn config_snapshot_mut(&self, py: Python<'_>) -> PyResult<ConfigSnapshotMut> {
        let lease = self.handle.acquire_mutation()?;
        runtime::run(py, "configuration snapshot", None, None, move |_| {
            let file = lease.handle.with(|r| Ok(r.config_snapshot().plumbing().clone()))?;
            Ok::<_, PyErr>(ConfigSnapshotMut {
                inner: Mutex::new(Some(Edit { lease, file })),
            })
        })?
    }
    fn config_file_mut(&self, py: Python<'_>, path: PathBuf) -> PyResult<ConfigFileTransaction> {
        self.handle.run(py, move |r| {
            r.config_file_mut(path)
                .map(|v| ConfigFileTransaction {
                    inner: Mutex::new(Some(v)),
                })
                .map_err(to_py)
        })
    }
    fn config_path(&self, py: Python<'_>, source: &str) -> PyResult<PathBuf> {
        let source = source_kind(source)?;
        self.handle.run(py, move |r| r.config_path(source).map_err(to_py))
    }
    fn author(&self, py: Python<'_>) -> PyResult<Option<Signature>> {
        self.handle.run(py, |r| {
            r.author()
                .map(|v| {
                    v.and_then(|v| v.to_owned())
                        .map(|inner| Signature { inner })
                        .map_err(to_py)
                })
                .transpose()
        })
    }
    fn committer(&self, py: Python<'_>) -> PyResult<Option<Signature>> {
        self.handle.run(py, |r| {
            r.committer()
                .map(|v| {
                    v.and_then(|v| v.to_owned())
                        .map(|inner| Signature { inner })
                        .map_err(to_py)
                })
                .transpose()
        })
    }
    fn big_file_threshold(&self, py: Python<'_>) -> PyResult<u64> {
        self.handle.run(py, |r| r.big_file_threshold().map_err(to_py))
    }
    fn open_options(&self, py: Python<'_>) -> PyResult<crate::repository::OpenOptions> {
        self.handle.run(py, |r| {
            Ok(crate::repository::OpenOptions {
                inner: r.open_options().clone(),
            })
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ConfigSnapshot>()?;
    m.add_class::<ConfigFile>()?;
    m.add_class::<ConfigSnapshotMut>()?;
    m.add_class::<ConfigRollback>()?;
    m.add_class::<ConfigFileTransaction>()?;
    Ok(())
}
