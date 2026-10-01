use crate::{error::to_py, objects::Signature, repository::Repository, types::bytes};
use gix::bstr::ByteSlice;
use pyo3::prelude::*;
use std::{ffi::OsString, path::PathBuf};

#[pyclass(frozen, module = "gix")]
pub struct Compression {
    inner: gix::zlib::Compression,
}
#[pymethods]
impl Compression {
    fn level(&self) -> i32 {
        self.inner.level()
    }
}

#[pyclass(frozen, module = "gix")]
pub struct FilesystemCapabilities {
    #[pyo3(get)]
    precompose_unicode: bool,
    #[pyo3(get)]
    ignore_case: bool,
    #[pyo3(get)]
    executable_bit: bool,
    #[pyo3(get)]
    symlink: bool,
}

#[cfg(feature = "index")]
#[pyclass(frozen, module = "gix")]
pub struct StatOptions {
    #[pyo3(get)]
    trust_ctime: bool,
    #[pyo3(get)]
    check_stat: bool,
    #[pyo3(get)]
    use_nsec: bool,
    #[pyo3(get)]
    use_stdev: bool,
}

#[cfg(feature = "attributes")]
#[pyclass(frozen, module = "gix")]
pub struct IgnorePatternParser {
    #[pyo3(get)]
    support_precious: bool,
}

#[cfg(feature = "network")]
#[pyclass(frozen, module = "gix")]
pub struct SshConnectOptions {
    #[pyo3(get)]
    command: Option<OsString>,
    #[pyo3(get)]
    disallow_shell: bool,
    #[pyo3(get)]
    kind: Option<String>,
}

#[cfg(feature = "attributes")]
#[pyclass(frozen, module = "gix")]
pub struct CommandContext {
    inner: gix::command::Context,
}
#[cfg(feature = "attributes")]
#[pymethods]
impl CommandContext {
    #[getter]
    fn git_dir(&self) -> Option<PathBuf> {
        self.inner.git_dir.clone()
    }
    #[getter]
    fn worktree_dir(&self) -> Option<PathBuf> {
        self.inner.worktree_dir.clone()
    }
    #[getter]
    fn no_replace_objects(&self) -> Option<bool> {
        self.inner.no_replace_objects
    }
    #[getter]
    fn ref_namespace<'py>(&self, py: Python<'py>) -> Option<Bound<'py, pyo3::types::PyBytes>> {
        self.inner
            .ref_namespace
            .as_ref()
            .map(|v| pyo3::types::PyBytes::new(py, v))
    }
    #[getter]
    fn literal_pathspecs(&self) -> Option<bool> {
        self.inner.literal_pathspecs
    }
    #[getter]
    fn glob_pathspecs(&self) -> Option<bool> {
        self.inner.glob_pathspecs
    }
    #[getter]
    fn icase_pathspecs(&self) -> Option<bool> {
        self.inner.icase_pathspecs
    }
    #[getter]
    fn stderr(&self) -> Option<bool> {
        self.inner.stderr
    }
}

#[pymethods]
impl Repository {
    fn git_dir_trust(&self, py: Python<'_>) -> PyResult<String> {
        self.handle.run(py, |r| Ok(format!("{:?}", r.git_dir_trust())))
    }
    #[cfg(feature = "attributes")]
    fn modules_path(&self, py: Python<'_>) -> PyResult<Option<PathBuf>> {
        self.handle.run(py, |r| Ok(r.modules_path()))
    }
    fn install_dir(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.handle.run(py, |r| r.install_dir().map_err(to_py))
    }
    fn committer_or_set_fallback(
        &self,
        py: Python<'_>,
        name: &Bound<'_, PyAny>,
        email: &Bound<'_, PyAny>,
    ) -> PyResult<Signature> {
        let name = bytes(name)?;
        let email = bytes(email)?;
        self.handle.mutate(py, move |r| {
            r.committer_or_set_fallback(name.as_bstr(), email.as_bstr())
                .and_then(|s| s.to_owned())
                .map(|inner| Signature { inner })
                .map_err(to_py)
        })
    }
    fn committer_or_set_generic_fallback(&self, py: Python<'_>) -> PyResult<Signature> {
        self.handle.mutate(py, |r| {
            r.committer_or_set_generic_fallback()
                .and_then(|s| s.to_owned())
                .map(|inner| Signature { inner })
                .map_err(to_py)
        })
    }
    fn loose_compression(&self, py: Python<'_>) -> PyResult<Compression> {
        self.handle.run(py, |r| {
            Ok(Compression {
                inner: r.loose_compression(),
            })
        })
    }
    fn pack_compression(&self, py: Python<'_>) -> PyResult<Compression> {
        self.handle.run(py, |r| {
            r.pack_compression().map(|inner| Compression { inner }).map_err(to_py)
        })
    }
    fn editor(&self, py: Python<'_>) -> PyResult<Option<OsString>> {
        self.handle.run(py, |r| Ok(r.editor()))
    }
    fn filesystem_options(&self, py: Python<'_>) -> PyResult<FilesystemCapabilities> {
        self.handle.run(py, |r| {
            r.filesystem_options()
                .map(|c| FilesystemCapabilities {
                    precompose_unicode: c.precompose_unicode,
                    ignore_case: c.ignore_case,
                    executable_bit: c.executable_bit,
                    symlink: c.symlink,
                })
                .map_err(to_py)
        })
    }
    #[cfg(feature = "index")]
    fn stat_options(&self, py: Python<'_>) -> PyResult<StatOptions> {
        self.handle.run(py, |r| {
            r.stat_options()
                .map(|s| StatOptions {
                    trust_ctime: s.trust_ctime,
                    check_stat: s.check_stat,
                    use_nsec: s.use_nsec,
                    use_stdev: s.use_stdev,
                })
                .map_err(to_py)
        })
    }
    #[cfg(feature = "index")]
    fn compute_object_cache_size_for_tree_diffs(
        &self,
        py: Python<'_>,
        index: &crate::index::IndexFile,
    ) -> PyResult<usize> {
        let index = index.snapshot()?;
        self.handle
            .run(py, move |r| Ok(r.compute_object_cache_size_for_tree_diffs(&index)))
    }
    #[cfg(feature = "attributes")]
    fn ignore_pattern_parser(&self, py: Python<'_>) -> PyResult<IgnorePatternParser> {
        self.handle.run(py, |r| {
            r.ignore_pattern_parser()
                .map(|v| IgnorePatternParser {
                    support_precious: v.support_precious,
                })
                .map_err(to_py)
        })
    }
    #[cfg(feature = "attributes")]
    fn command_context(&self, py: Python<'_>) -> PyResult<CommandContext> {
        self.handle.run(py, |r| {
            r.command_context().map(|inner| CommandContext { inner }).map_err(to_py)
        })
    }
    #[cfg(feature = "blob-diff")]
    fn diff_algorithm(&self, py: Python<'_>) -> PyResult<String> {
        self.handle
            .run(py, |r| r.diff_algorithm().map(|v| format!("{v:?}")).map_err(to_py))
    }
    #[cfg(feature = "network")]
    fn ssh_connect_options(&self, py: Python<'_>) -> PyResult<SshConnectOptions> {
        self.handle.run(py, |r| {
            r.ssh_connect_options()
                .map(|v| SshConnectOptions {
                    command: v.command,
                    disallow_shell: v.disallow_shell,
                    kind: v.kind.map(|k| format!("{k:?}")),
                })
                .map_err(to_py)
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Compression>()?;
    m.add_class::<FilesystemCapabilities>()?;
    #[cfg(feature = "index")]
    m.add_class::<StatOptions>()?;
    #[cfg(feature = "attributes")]
    m.add_class::<IgnorePatternParser>()?;
    #[cfg(feature = "attributes")]
    m.add_class::<CommandContext>()?;
    #[cfg(feature = "network")]
    m.add_class::<SshConnectOptions>()?;
    Ok(())
}
