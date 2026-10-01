//! Worktree discovery, checkout, and removal through native Gitoxide operations.
#![allow(
    clippy::wrong_self_convention,
    reason = "Preserve native names for Python-owned values"
)]

use crate::{
    error::to_py,
    repository::{RepoHandle, Repository},
    runtime::{self, CancellationToken, OwnedIter, Progress},
    types::bytes,
};
use gix::bstr::ByteSlice;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use std::path::PathBuf;

#[cfg(feature = "worktree-stream")]
#[path = "worktree_stream.rs"]
mod stream;

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Worktree {
    handle: RepoHandle,
}
impl Worktree {
    fn with<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(gix::Worktree<'_>) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        self.handle.run(py, move |repo| {
            work(
                repo.worktree()
                    .ok_or_else(|| PyValueError::new_err("repository has no worktree"))?,
            )
        })
    }
}
#[pymethods]
impl Worktree {
    fn base(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.with(py, |w| Ok(w.base().into()))
    }
    fn is_main(&self, py: Python<'_>) -> PyResult<bool> {
        self.with(py, |w| Ok(w.is_main()))
    }
    fn is_locked(&self, py: Python<'_>) -> PyResult<bool> {
        self.with(py, |w| Ok(w.is_locked()))
    }
    fn dot_git_exists(&self, py: Python<'_>) -> PyResult<bool> {
        self.with(py, |w| Ok(w.dot_git_exists()))
    }
    fn id<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        Ok(self
            .with(py, |w| Ok(w.id().map(|n| n.to_vec())))?
            .map(|v| PyBytes::new(py, &v)))
    }
    fn lock_reason<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        Ok(self.with(py, |w| Ok(w.lock_reason()))?.map(|v| PyBytes::new(py, &v)))
    }
    #[cfg(feature = "index")]
    fn open_index(&self, py: Python<'_>) -> PyResult<crate::index::IndexFile> {
        self.with(py, |w| {
            w.open_index().map(crate::index::IndexFile::from_native).map_err(to_py)
        })
    }
    #[cfg(feature = "index")]
    fn index(&self, py: Python<'_>) -> PyResult<crate::index::IndexFile> {
        self.with(py, |w| {
            w.index().map(crate::index::IndexFile::from_shared).map_err(to_py)
        })
    }
    #[cfg(feature = "attributes")]
    #[pyo3(signature=(overrides=None))]
    fn attributes(
        &self,
        py: Python<'_>,
        overrides: Option<&crate::attributes::IgnoreSearch>,
    ) -> PyResult<crate::attributes::AttributeStack> {
        let handle = self.handle.clone();
        let overrides = overrides.map(|v| v.inner.clone());
        self.with(py, move |w| {
            w.attributes(overrides)
                .map(|native| crate::attributes::AttributeStack::from_native(handle, native.detach()))
                .map_err(to_py)
        })
    }
    #[cfg(feature = "attributes")]
    fn attributes_only(&self, py: Python<'_>) -> PyResult<crate::attributes::AttributeStack> {
        let handle = self.handle.clone();
        self.with(py, move |w| {
            w.attributes_only()
                .map(|native| crate::attributes::AttributeStack::from_native(handle, native.detach()))
                .map_err(to_py)
        })
    }
    #[cfg(feature = "attributes")]
    fn pathspec(&self, py: Python<'_>, patterns: Vec<Bound<'_, PyAny>>) -> PyResult<crate::pathspec::Pathspec> {
        let patterns = patterns.iter().map(bytes).collect::<PyResult<Vec<_>>>()?;
        self.with(py, move |w| {
            w.pathspec(patterns.iter().map(|p| p.as_bstr()))
                .and_then(|native| native.detach())
                .map(crate::pathspec::Pathspec::from_native)
                .map_err(to_py)
        })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct WorktreeProxy {
    handle: RepoHandle,
    id: Vec<u8>,
    git_dir: PathBuf,
}
impl WorktreeProxy {
    fn from_native(handle: RepoHandle, proxy: gix::worktree::Proxy<'_>) -> Self {
        Self {
            handle,
            id: proxy.id().to_vec(),
            git_dir: proxy.git_dir().into(),
        }
    }
    fn with<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(gix::worktree::Proxy<'_>) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let id = self.id.clone();
        self.handle.run(py, move |repo| {
            work(
                repo.worktree_proxy_by_id(id.as_bstr())
                    .ok_or_else(|| PyValueError::new_err("worktree registration is no longer available"))?,
            )
        })
    }
}
#[pymethods]
impl WorktreeProxy {
    fn git_dir(&self) -> PathBuf {
        self.git_dir.clone()
    }
    fn id<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.id)
    }
    fn base(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.with(py, |p| p.base().map_err(to_py))
    }
    fn is_locked(&self, py: Python<'_>) -> PyResult<bool> {
        self.with(py, |p| Ok(p.is_locked()))
    }
    fn is_prunable(&self, py: Python<'_>) -> PyResult<bool> {
        self.with(py, |p| Ok(p.is_prunable()))
    }
    fn lock_reason<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        Ok(self.with(py, |p| Ok(p.lock_reason()))?.map(|v| PyBytes::new(py, &v)))
    }
    fn into_repo(&self, py: Python<'_>) -> PyResult<Repository> {
        self.with(py, |p| p.into_repo().map(Repository::from_native).map_err(to_py))
    }
    fn into_repo_with_possibly_inaccessible_worktree(&self, py: Python<'_>) -> PyResult<Repository> {
        self.with(py, |p| {
            p.into_repo_with_possibly_inaccessible_worktree()
                .map(Repository::from_native)
                .map_err(to_py)
        })
    }
    #[cfg(feature = "worktree-mutation")]
    #[pyo3(signature=(force=None,*,progress=None))]
    fn remove(&self, py: Python<'_>, force: Option<WorktreeRemoveForce>, progress: Option<&Progress>) -> PyResult<()> {
        let handle = self.handle.clone();
        let id = self.id.clone();
        runtime::run(py, "remove worktree", progress, None, move |ctx| {
            handle.with(|r| {
                r.worktree_proxy_by_id(id.as_bstr())
                    .ok_or_else(|| PyValueError::new_err("worktree registration is no longer available"))?
                    .remove(force.unwrap_or_default().inner, ctx.progress)
                    .map_err(to_py)
            })
        })?
    }
}

#[pyclass(frozen, module = "gix")]
pub struct WorktreeRepositoryIter {
    inner: OwnedIter<Repository, PyErr>,
}
#[pymethods]
impl WorktreeRepositoryIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Repository>> {
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
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}
#[pymethods]
impl Repository {
    fn worktree(&self, py: Python<'_>) -> PyResult<Option<Worktree>> {
        self.handle.run(py, |r| {
            Ok(r.worktree().map(|_| Worktree {
                handle: RepoHandle::new(r.clone()),
            }))
        })
    }
    /// Native worktrees() computes and sorts the complete proxy list.
    fn worktrees(&self, py: Python<'_>) -> PyResult<Vec<WorktreeProxy>> {
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.worktrees()
                .map(|v| {
                    v.into_iter()
                        .map(|p| WorktreeProxy::from_native(handle.clone(), p))
                        .collect()
                })
                .map_err(to_py)
        })
    }
    #[pyo3(signature=(*,progress=None,cancel=None))]
    fn worktrees_including_main(
        &self,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> WorktreeRepositoryIter {
        let handle = self.handle.clone();
        WorktreeRepositoryIter {
            inner: OwnedIter::new("worktrees", progress, cancel, move |_, producer| {
                handle.with(|r| {
                    producer.serve(
                        r.worktrees_including_main()
                            .map_err(to_py)?
                            .map(|repo| repo.map(Repository::from_native).map_err(to_py)),
                    )
                })
            }),
        }
    }
    fn worktree_proxy_by_id(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Option<WorktreeProxy>> {
        let id = bytes(id)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            Ok(r.worktree_proxy_by_id(id.as_bstr())
                .map(|p| WorktreeProxy::from_native(handle, p)))
        })
    }
    fn main_repo(&self, py: Python<'_>) -> PyResult<Repository> {
        self.handle
            .run(py, |r| r.main_repo().map(Repository::from_native).map_err(to_py))
    }
}

#[cfg(feature = "worktree-mutation")]
mod mutation {
    use super::*;
    use crate::types::ObjectId;
    use std::sync::Arc;

    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct WorktreeHead {
        inner: gix::worktree::add::Head,
    }
    #[pymethods]
    impl WorktreeHead {
        #[staticmethod]
        #[pyo3(name = "Attached")]
        fn attached(name: &Bound<'_, PyAny>) -> PyResult<Self> {
            Ok(Self {
                inner: gix::worktree::add::Head::Attached(
                    gix::refs::FullName::try_from(bytes(name)?.as_bstr()).map_err(to_py)?,
                ),
            })
        }
        #[staticmethod]
        #[pyo3(name = "Detached")]
        fn detached(id: ObjectId) -> Self {
            Self {
                inner: gix::worktree::add::Head::Detached(id.inner),
            }
        }
    }
    #[pyclass(frozen, module = "gix", from_py_object, eq)]
    #[derive(Clone, Copy, Default, PartialEq, Eq)]
    pub struct WorktreeRemoveForce {
        pub inner: gix::worktree::remove::Force,
    }
    #[pymethods]
    impl WorktreeRemoveForce {
        #[classattr]
        #[pyo3(name = "Never")]
        fn never() -> Self {
            Self::default()
        }
        #[classattr]
        #[pyo3(name = "DiscardChanges")]
        fn discard_changes() -> Self {
            Self {
                inner: gix::worktree::remove::Force::DiscardChanges,
            }
        }
        #[classattr]
        #[pyo3(name = "OverrideLock")]
        fn override_lock() -> Self {
            Self {
                inner: gix::worktree::remove::Force::OverrideLock,
            }
        }
    }
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct CheckoutOutcome {
        pub inner: Arc<gix::worktree::state::checkout::Outcome>,
    }
    #[pymethods]
    impl CheckoutOutcome {
        #[getter]
        fn files_updated(&self) -> usize {
            self.inner.files_updated
        }
        #[getter]
        fn bytes_written(&self) -> u64 {
            self.inner.bytes_written
        }
        #[getter]
        fn collisions(&self, py: Python<'_>) -> Vec<(Py<PyBytes>, String)> {
            self.inner
                .collisions
                .iter()
                .map(|v| (PyBytes::new(py, &v.path).unbind(), v.error_kind.to_string()))
                .collect()
        }
        #[getter]
        fn errors(&self, py: Python<'_>) -> Vec<(Py<PyBytes>, String)> {
            self.inner
                .errors
                .iter()
                .map(|v| (PyBytes::new(py, &v.path).unbind(), v.error.to_string()))
                .collect()
        }
        #[getter]
        fn delayed_paths_unknown(&self, py: Python<'_>) -> Vec<Py<PyBytes>> {
            self.inner
                .delayed_paths_unknown
                .iter()
                .map(|v| PyBytes::new(py, v).unbind())
                .collect()
        }
        #[getter]
        fn delayed_paths_unprocessed(&self, py: Python<'_>) -> Vec<Py<PyBytes>> {
            self.inner
                .delayed_paths_unprocessed
                .iter()
                .map(|v| PyBytes::new(py, v).unbind())
                .collect()
        }
    }
    #[pymethods]
    impl Repository {
        #[pyo3(signature=(destination,head,*,progress=None,cancel=None))]
        fn add_worktree(
            &self,
            py: Python<'_>,
            destination: PathBuf,
            head: WorktreeHead,
            progress: Option<&Progress>,
            cancel: Option<&CancellationToken>,
        ) -> PyResult<(Repository, CheckoutOutcome)> {
            let handle = self.handle.clone();
            runtime::run(py, "add worktree", progress, cancel, move |ctx| {
                handle.with(|r| {
                    if let gix::worktree::add::Head::Detached(id) = &head.inner
                        && id.kind() != r.object_hash()
                    {
                        return Err(PyValueError::new_err(
                            "worktree target hash kind differs from repository",
                        ));
                    }
                    r.add_worktree(destination, head.inner, ctx.progress, &ctx.interrupt)
                        .map(|(repo, inner)| {
                            (
                                Repository::from_native(repo),
                                CheckoutOutcome { inner: Arc::new(inner) },
                            )
                        })
                        .map_err(to_py)
                })
            })?
        }
        #[pyo3(signature=(target,force=None,*,progress=None))]
        fn remove_worktree(
            &self,
            py: Python<'_>,
            target: PathBuf,
            force: Option<WorktreeRemoveForce>,
            progress: Option<&Progress>,
        ) -> PyResult<()> {
            let handle = self.handle.clone();
            runtime::run(py, "remove worktree", progress, None, move |ctx| {
                handle.with(|r| {
                    r.remove_worktree(target, force.unwrap_or_default().inner, ctx.progress)
                        .map_err(to_py)
                })
            })?
        }
    }
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone, Copy, Default)]
    pub struct WorktreeRemoveOptions {
        inner: gix::worktree::remove::Options,
    }
    #[pymethods]
    impl WorktreeRemoveOptions {
        #[new]
        #[pyo3(signature=(*,thread_limit=None,max_retries=2))]
        fn new(thread_limit: Option<usize>, max_retries: usize) -> Self {
            Self {
                inner: gix::worktree::remove::Options {
                    thread_limit,
                    max_retries,
                },
            }
        }
    }
    enum RemoveCommand {
        Base,
        Repository,
        Options(WorktreeRemoveOptions),
        Remove(WorktreeRemoveForce),
    }
    enum RemoveReply {
        Base(PathBuf),
        Repository(Repository),
        Done,
    }
    #[pyclass(frozen, module = "gix")]
    pub struct WorktreeRemoveTarget {
        owner: runtime::CommandOwner<RemoveCommand, RemoveReply>,
        base: PathBuf,
    }
    #[pymethods]
    impl WorktreeRemoveTarget {
        fn base(&self) -> PathBuf {
            self.base.clone()
        }
        fn repository(&self, py: Python<'_>) -> PyResult<Repository> {
            match self.owner.call(py, RemoveCommand::Repository)? {
                RemoveReply::Repository(repo) => Ok(repo),
                _ => Err(PyValueError::new_err("unexpected worktree response")),
            }
        }
        fn options<'py>(
            slf: PyRef<'py, Self>,
            py: Python<'py>,
            options: WorktreeRemoveOptions,
        ) -> PyResult<PyRef<'py, Self>> {
            slf.owner.call(py, RemoveCommand::Options(options))?;
            Ok(slf)
        }
        #[pyo3(signature=(force=None))]
        fn remove(&self, py: Python<'_>, force: Option<WorktreeRemoveForce>) -> PyResult<()> {
            self.owner.call(py, RemoveCommand::Remove(force.unwrap_or_default()))?;
            self.owner.finish(py)
        }
        fn close(&self, py: Python<'_>) -> PyResult<()> {
            self.owner.close(py)
        }
        fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
            slf
        }
        fn __exit__(
            &self,
            py: Python<'_>,
            _exc_type: &Bound<'_, PyAny>,
            _exc_value: &Bound<'_, PyAny>,
            _traceback: &Bound<'_, PyAny>,
        ) -> PyResult<()> {
            self.close(py)
        }
    }
    #[pymethods]
    impl Repository {
        #[pyo3(signature=(target,*,progress=None))]
        fn prepare_remove_worktree(
            &self,
            py: Python<'_>,
            target: PathBuf,
            progress: Option<&Progress>,
        ) -> PyResult<WorktreeRemoveTarget> {
            let handle = self.handle.clone();
            let owner = runtime::CommandOwner::new_with_options(
                "remove worktree",
                progress,
                None,
                move |ctx, commands, producer| {
                    handle.with(|repo| {
                        let mut target = Some(repo.prepare_remove_worktree(target).map_err(to_py)?);
                        while producer.requested() {
                            let command = commands
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .take()
                                .ok_or_else(|| PyValueError::new_err("missing worktree command"))?;
                            let Some(native) = target.as_ref() else {
                                return Err(PyValueError::new_err("worktree removal target has been consumed"));
                            };
                            let reply = match command {
                                RemoveCommand::Base => Ok(RemoveReply::Base(native.base().into())),
                                RemoveCommand::Repository => native
                                    .repository()
                                    .map(Repository::from_native)
                                    .map(RemoveReply::Repository)
                                    .map_err(to_py),
                                RemoveCommand::Options(options) => {
                                    target = target.take().map(|target| target.options(options.inner));
                                    Ok(RemoveReply::Done)
                                }
                                RemoveCommand::Remove(force) => {
                                    let target = target.take().ok_or_else(|| {
                                        PyValueError::new_err("worktree removal target has been consumed")
                                    })?;
                                    target.remove(force.inner, ctx.progress).map_err(to_py)?;
                                    producer.send(Ok(RemoveReply::Done));
                                    return Ok(());
                                }
                            };
                            if !producer.send(reply) {
                                break;
                            }
                        }
                        Ok(())
                    })
                },
            );
            let RemoveReply::Base(base) = owner.call(py, RemoveCommand::Base)? else {
                return Err(PyValueError::new_err("unexpected worktree response"));
            };
            Ok(WorktreeRemoveTarget { owner, base })
        }
    }

    pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<WorktreeRemoveOptions>()?;
        m.add_class::<WorktreeRemoveTarget>()?;
        m.add_class::<WorktreeHead>()?;
        m.add_class::<WorktreeRemoveForce>()?;
        m.add_class::<CheckoutOutcome>()?;
        Ok(())
    }
}
#[cfg(all(feature = "worktree-mutation", feature = "network"))]
pub use mutation::CheckoutOutcome;
#[cfg(feature = "worktree-mutation")]
use mutation::WorktreeRemoveForce;

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Worktree>()?;
    m.add_class::<WorktreeProxy>()?;
    m.add_class::<WorktreeRepositoryIter>()?;
    #[cfg(feature = "worktree-mutation")]
    mutation::register(m)?;
    #[cfg(feature = "worktree-stream")]
    stream::register(m)?;
    Ok(())
}
