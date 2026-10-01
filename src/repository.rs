use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use gix::bstr::ByteSlice;
use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyBytes};

use crate::{
    error::to_py,
    runtime,
    types::{HashKind, bytes},
};

struct State {
    in_memory: bool,
    sync: RwLock<gix::ThreadSafeRepository>,
    memory: Mutex<Option<gix::Repository>>,
    cache: RwLock<Option<Option<usize>>>,
    mutation: AtomicBool,
}

#[derive(Clone)]
pub struct RepoHandle(Arc<State>);

pub struct MutationLease {
    pub handle: RepoHandle,
}
impl Drop for MutationLease {
    fn drop(&mut self) {
        self.handle.0.mutation.store(false, Ordering::Release);
    }
}
impl MutationLease {
    pub fn apply<T>(&self, work: impl FnOnce(&mut gix::Repository) -> PyResult<T>) -> PyResult<T> {
        self.handle.with_mut_unchecked(work)
    }
}

impl RepoHandle {
    pub fn new(repo: gix::Repository) -> Self {
        Self(Arc::new(State {
            in_memory: false,
            sync: RwLock::new(repo.into_sync()),
            memory: Mutex::new(None),
            cache: RwLock::new(None),
            mutation: AtomicBool::new(false),
        }))
    }
    pub fn with<T>(&self, work: impl FnOnce(&gix::Repository) -> PyResult<T>) -> PyResult<T> {
        if self.0.in_memory {
            let memory = self
                .0
                .memory
                .try_lock()
                .map_err(|_| PyRuntimeError::new_err("in-memory repository is already in use"))?;
            let repo = memory
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("in-memory repository is unavailable"))?;
            // ponytail: native in-memory object state is local; only these opt-in handles serialize operations.
            return work(repo);
        }
        let sync = self.0.sync.read().map_err(to_py)?.clone();
        let mut repo = sync.to_thread_local();
        if let Some(cache) = *self.0.cache.read().map_err(to_py)? {
            repo.object_cache_size(cache);
        }
        work(&repo)
    }
    fn set_cache(&self, py: Python<'_>, bytes: Option<usize>, only_if_unset: bool) -> PyResult<()> {
        let handle = self.clone();
        self.mutate(py, move |repo| {
            if !only_if_unset || !repo.objects.has_object_cache() {
                repo.object_cache_size(bytes);
                *handle.0.cache.write().map_err(to_py)? = Some(bytes);
            }
            Ok(())
        })
    }
    pub fn run<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&gix::Repository) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let handle = self.clone();
        runtime::run(py, "repository", None, None, move |_| handle.with(work))?
    }
    pub fn mutate<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::Repository) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let handle = self.clone();
        runtime::run(py, "repository mutation", None, None, move |_| handle.with_mut(work))?
    }
    pub fn with_mut<T>(&self, work: impl FnOnce(&mut gix::Repository) -> PyResult<T>) -> PyResult<T> {
        self.acquire_mutation()?.apply(work)
    }
    pub fn acquire_mutation(&self) -> PyResult<MutationLease> {
        if self
            .0
            .mutation
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(PyRuntimeError::new_err("repository is already being mutated"));
        }
        Ok(MutationLease { handle: self.clone() })
    }
    fn with_mut_unchecked<T>(&self, work: impl FnOnce(&mut gix::Repository) -> PyResult<T>) -> PyResult<T> {
        if self.0.in_memory {
            let mut memory = self
                .0
                .memory
                .try_lock()
                .map_err(|_| PyRuntimeError::new_err("in-memory repository is already in use"))?;
            let repo = memory
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("in-memory repository is unavailable"))?;
            return work(repo);
        }
        let mut repo = self.0.sync.read().map_err(to_py)?.to_thread_local();
        if let Some(cache) = *self.0.cache.read().map_err(to_py)? {
            repo.object_cache_size(cache);
        }
        let result = work(&mut repo);
        *self.0.sync.write().map_err(to_py)? = repo.into_sync();
        result
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Repository {
    pub handle: RepoHandle,
}

impl Repository {
    pub fn from_native(repo: gix::Repository) -> Self {
        Self {
            handle: RepoHandle::new(repo),
        }
    }
}

#[pymethods]
impl Repository {
    fn git_dir(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.handle.run(py, |r| Ok(r.git_dir().into()))
    }
    fn path(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.git_dir(py)
    }
    fn common_dir(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.handle.run(py, |r| Ok(r.common_dir().into()))
    }
    fn current_dir(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.handle.run(py, |r| Ok(r.current_dir().into()))
    }
    fn workdir(&self, py: Python<'_>) -> PyResult<Option<PathBuf>> {
        self.handle.run(py, |r| Ok(r.workdir().map(Into::into)))
    }
    fn work_dir(&self, py: Python<'_>) -> PyResult<Option<PathBuf>> {
        self.workdir(py)
    }
    fn index_path(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.handle.run(py, |r| Ok(r.index_path()))
    }
    fn prefix(&self, py: Python<'_>) -> PyResult<Option<PathBuf>> {
        self.handle
            .run(py, |r| r.prefix().map(|v| v.map(Into::into)).map_err(to_py))
    }
    fn is_bare(&self, py: Python<'_>) -> PyResult<bool> {
        self.handle.run(py, |r| Ok(r.is_bare()))
    }
    fn is_pristine(&self, py: Python<'_>) -> PyResult<Option<bool>> {
        self.handle.run(py, |r| Ok(r.is_pristine()))
    }
    fn kind(&self, py: Python<'_>) -> PyResult<String> {
        self.handle.run(py, |r| Ok(format!("{:?}", r.kind())))
    }
    fn state(&self, py: Python<'_>) -> PyResult<Option<String>> {
        self.handle.run(py, |r| Ok(r.state().map(|v| format!("{v:?}"))))
    }
    fn object_hash(&self, py: Python<'_>) -> PyResult<HashKind> {
        self.handle.run(py, |r| Ok(HashKind { inner: r.object_hash() }))
    }
    fn workdir_path(&self, py: Python<'_>, rela_path: &Bound<'_, PyAny>) -> PyResult<Option<PathBuf>> {
        let path = bytes(rela_path)?;
        self.handle.run(py, move |r| Ok(r.workdir_path(path.as_bstr())))
    }
    fn normalize_path<'py>(&self, py: Python<'py>, path: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyBytes>> {
        let path = bytes(path)?;
        let result = self.handle.run(py, move |r| {
            r.normalize_path(path.as_bstr()).map(|v| v.to_vec()).map_err(to_py)
        })?;
        Ok(PyBytes::new(py, &result))
    }
    fn set_workdir(&self, py: Python<'_>, workdir: Option<PathBuf>) -> PyResult<Option<PathBuf>> {
        self.handle.mutate(py, move |r| r.set_workdir(workdir).map_err(to_py))
    }
    fn reload<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<PyRef<'py, Self>> {
        let handle = slf.handle.clone();
        slf.handle.mutate(py, move |r| {
            r.reload().map_err(to_py)?;
            *handle.0.cache.write().map_err(to_py)? = None;
            Ok(())
        })?;
        Ok(slf)
    }
    fn object_cache_size(&self, py: Python<'_>, bytes: Option<usize>) -> PyResult<()> {
        self.handle.set_cache(py, bytes, false)
    }
    fn object_cache_size_if_unset(&self, py: Python<'_>, bytes: usize) -> PyResult<()> {
        self.handle.set_cache(py, Some(bytes), true)
    }
    fn with_object_memory(&self, py: Python<'_>) -> PyResult<Self> {
        self.handle.run(py, |r| {
            let sync = r.clone().into_sync();
            let memory = r.clone().with_object_memory();
            Ok(Self {
                handle: RepoHandle(Arc::new(State {
                    in_memory: true,
                    sync: RwLock::new(sync),
                    memory: Mutex::new(Some(memory)),
                    cache: RwLock::new(None),
                    mutation: AtomicBool::new(false),
                })),
            })
        })
    }
    fn __repr__(&self) -> PyResult<String> {
        self.handle.with(|r| Ok(format!("Repository({:?})", r.git_dir())))
    }
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct OpenOptions {
    pub inner: gix::open::Options,
}

#[pymethods]
impl OpenOptions {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    #[staticmethod]
    fn isolated() -> Self {
        Self {
            inner: gix::open::Options::isolated(),
        }
    }
    fn config_overrides(&self, values: Vec<String>) -> Self {
        Self {
            inner: self.inner.clone().config_overrides(values),
        }
    }
    fn cli_overrides(&self, values: Vec<String>) -> Self {
        Self {
            inner: self.inner.clone().cli_overrides(values),
        }
    }
    fn open_path_as_is(&self, enable: bool) -> Self {
        Self {
            inner: self.inner.clone().open_path_as_is(enable),
        }
    }
    fn bail_if_untrusted(&self, enable: bool) -> Self {
        Self {
            inner: self.inner.clone().bail_if_untrusted(enable),
        }
    }
    fn strict_config(&self, enable: bool) -> Self {
        Self {
            inner: self.inner.clone().strict_config(enable),
        }
    }
    fn lossy_config(&self, enable: bool) -> Self {
        Self {
            inner: self.inner.clone().lossy_config(enable),
        }
    }
    fn open(&self, py: Python<'_>, path: PathBuf) -> PyResult<Repository> {
        open_opts(py, path, self.clone())
    }
}

#[pyfunction]
fn open(py: Python<'_>, path: PathBuf) -> PyResult<Repository> {
    runtime::run(py, "open", None, None, move |_| {
        gix::open(path).map(Repository::from_native).map_err(to_py)
    })?
}
#[pyfunction]
fn open_opts(py: Python<'_>, path: PathBuf, options: OpenOptions) -> PyResult<Repository> {
    runtime::run(py, "open", None, None, move |_| {
        gix::open_opts(path, options.inner)
            .map(Repository::from_native)
            .map_err(to_py)
    })?
}
#[pyfunction]
fn discover(py: Python<'_>, path: PathBuf) -> PyResult<Repository> {
    runtime::run(py, "discover", None, None, move |_| {
        gix::discover(path).map(Repository::from_native).map_err(to_py)
    })?
}
#[pyfunction]
fn discover_opts(py: Python<'_>, path: PathBuf, options: OpenOptions) -> PyResult<Repository> {
    runtime::run(py, "discover", None, None, move |_| {
        gix::discover_opts(path, Default::default(), options.inner)
            .map(Repository::from_native)
            .map_err(to_py)
    })?
}
#[pyfunction]
#[pyo3(signature = (path, *, object_hash=None, options=None, destination_must_be_empty=None))]
fn init(
    py: Python<'_>,
    path: PathBuf,
    object_hash: Option<HashKind>,
    options: Option<OpenOptions>,
    destination_must_be_empty: Option<bool>,
) -> PyResult<Repository> {
    init_inner(
        py,
        path,
        gix::create::Kind::WithWorktree,
        object_hash,
        options,
        destination_must_be_empty,
    )
}
#[pyfunction]
#[pyo3(signature = (path, *, object_hash=None, options=None))]
fn init_bare(
    py: Python<'_>,
    path: PathBuf,
    object_hash: Option<HashKind>,
    options: Option<OpenOptions>,
) -> PyResult<Repository> {
    init_inner(py, path, gix::create::Kind::Bare, object_hash, options, None)
}
fn init_inner(
    py: Python<'_>,
    path: PathBuf,
    kind: gix::create::Kind,
    hash: Option<HashKind>,
    options: Option<OpenOptions>,
    empty: Option<bool>,
) -> PyResult<Repository> {
    runtime::run(py, "init", None, None, move |_| {
        let mut create = gix::create::Options {
            destination_must_be_empty: empty,
            ..Default::default()
        };
        if let Some(hash) = hash {
            create.object_hash = Some(hash.inner);
        }
        gix::ThreadSafeRepository::init_opts(path, kind, create, options.unwrap_or_default().inner)
            .map(|r| Repository::from_native(r.to_thread_local()))
            .map_err(to_py)
    })?
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Repository>()?;
    m.add_class::<OpenOptions>()?;
    m.add_function(wrap_pyfunction!(open, m)?)?;
    m.add_function(wrap_pyfunction!(open_opts, m)?)?;
    m.add_function(wrap_pyfunction!(discover, m)?)?;
    m.add_function(wrap_pyfunction!(discover_opts, m)?)?;
    m.add_function(wrap_pyfunction!(init, m)?)?;
    m.add_function(wrap_pyfunction!(init_bare, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_policy_updates_persistent_memory_and_survives_operations() {
        Python::initialize();
        Python::attach(|py| {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("pygix-cache-{}-{stamp}", std::process::id()));
            let repo = Repository::from_native(gix::init_bare(&path).expect("temporary repository"));
            let memory = repo.with_object_memory(py).expect("memory view");
            for view in [&repo, &memory] {
                view.object_cache_size(py, None).expect("disable");
                assert!(!view.handle.with(|r| Ok(r.objects.has_object_cache())).expect("query"));
                view.object_cache_size_if_unset(py, 4096).expect("enable");
                assert!(view.handle.with(|r| Ok(r.objects.has_object_cache())).expect("query"));
                view.object_cache_size_if_unset(py, 0).expect("retain existing");
                assert!(view.handle.with(|r| Ok(r.objects.has_object_cache())).expect("query"));
            }
            drop((repo, memory));
            std::fs::remove_dir_all(path).expect("cleanup");
        });
    }
}
