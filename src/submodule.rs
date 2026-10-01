//! Native submodule snapshots and their shared, worker-owned lazy state.
use crate::{
    config::ConfigFile,
    error::to_py,
    repository::Repository,
    runtime::{CommandOwner, OwnedIter},
    types::{ObjectId, bytes},
};
use gix::bstr::ByteSlice;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use std::{
    any::Any,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubmoduleIgnore {
    pub inner: gix::submodule::config::Ignore,
}
#[pymethods]
impl SubmoduleIgnore {
    #[classattr]
    #[pyo3(name = "All")]
    fn all() -> Self {
        Self {
            inner: gix::submodule::config::Ignore::All,
        }
    }
    #[classattr]
    #[pyo3(name = "Dirty")]
    fn dirty() -> Self {
        Self {
            inner: gix::submodule::config::Ignore::Dirty,
        }
    }
    #[classattr]
    #[pyo3(name = "Untracked")]
    fn untracked() -> Self {
        Self {
            inner: gix::submodule::config::Ignore::Untracked,
        }
    }
    #[classattr]
    #[pyo3(name = "None_")]
    fn none() -> Self {
        Self {
            inner: gix::submodule::config::Ignore::None,
        }
    }
    fn __repr__(&self) -> String {
        format!("SubmoduleIgnore.{:?}", self.inner)
    }
}
#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubmoduleFetchRecurse {
    inner: gix::submodule::config::FetchRecurse,
}
#[pymethods]
impl SubmoduleFetchRecurse {
    #[classattr]
    #[pyo3(name = "OnDemand")]
    fn on_demand() -> Self {
        Self {
            inner: gix::submodule::config::FetchRecurse::OnDemand,
        }
    }
    #[classattr]
    #[pyo3(name = "Always")]
    fn always() -> Self {
        Self {
            inner: gix::submodule::config::FetchRecurse::Always,
        }
    }
    #[classattr]
    #[pyo3(name = "Never")]
    fn never() -> Self {
        Self {
            inner: gix::submodule::config::FetchRecurse::Never,
        }
    }
}
#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct SubmoduleUpdate {
    inner: gix::submodule::config::Update,
}
#[pymethods]
impl SubmoduleUpdate {
    #[classattr]
    #[pyo3(name = "Checkout")]
    fn checkout() -> Self {
        Self {
            inner: gix::submodule::config::Update::Checkout,
        }
    }
    #[classattr]
    #[pyo3(name = "Rebase")]
    fn rebase() -> Self {
        Self {
            inner: gix::submodule::config::Update::Rebase,
        }
    }
    #[classattr]
    #[pyo3(name = "Merge")]
    fn merge() -> Self {
        Self {
            inner: gix::submodule::config::Update::Merge,
        }
    }
    #[classattr]
    #[pyo3(name = "None_")]
    fn none() -> Self {
        Self {
            inner: gix::submodule::config::Update::None,
        }
    }
    #[staticmethod]
    #[pyo3(name = "Command")]
    fn command(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: gix::submodule::config::Update::Command(bytes(value)?.into()),
        })
    }
    fn command_bytes<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        match &self.inner {
            gix::submodule::config::Update::Command(data) => Some(PyBytes::new(py, data)),
            _ => None,
        }
    }
}
#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct SubmoduleBranch {
    inner: gix::submodule::config::Branch,
}
#[pymethods]
impl SubmoduleBranch {
    #[classattr]
    #[pyo3(name = "CurrentInSuperproject")]
    fn current() -> Self {
        Self {
            inner: gix::submodule::config::Branch::CurrentInSuperproject,
        }
    }
    #[staticmethod]
    #[pyo3(name = "Name")]
    fn name(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: gix::submodule::config::Branch::try_from(bytes(value)?.as_bstr()).map_err(to_py)?,
        })
    }
    fn name_bytes<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        match &self.inner {
            gix::submodule::config::Branch::Name(data) => Some(PyBytes::new(py, data)),
            _ => None,
        }
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Copy)]
pub struct SubmoduleState {
    inner: gix::submodule::State,
}
#[pymethods]
impl SubmoduleState {
    #[getter]
    fn repository_exists(&self) -> bool {
        self.inner.repository_exists
    }
    #[getter]
    fn is_old_form(&self) -> bool {
        self.inner.is_old_form
    }
    #[getter]
    fn worktree_checkout(&self) -> bool {
        self.inner.worktree_checkout
    }
    #[getter]
    fn superproject_configuration(&self) -> bool {
        self.inner.superproject_configuration
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct ModulesFile {
    inner: Arc<Mutex<gix::submodule::File>>,
}
impl ModulesFile {
    fn with<T: Send>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::submodule::File) -> PyResult<T> + Send,
    ) -> PyResult<T> {
        py.detach(|| work(&mut *self.inner.lock().map_err(to_py)?))
    }
}
#[pyclass(frozen, module = "gix")]
pub struct SubmoduleNames {
    inner: OwnedIter<Vec<u8>, PyErr>,
}
#[pymethods]
impl SubmoduleNames {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        Ok(self.inner.next(py)?.transpose()?.map(|v| PyBytes::new(py, &v)))
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
impl ModulesFile {
    #[staticmethod]
    #[pyo3(signature=(data,path=None,config=None))]
    fn from_bytes(
        py: Python<'_>,
        data: &Bound<'_, PyBytes>,
        path: Option<PathBuf>,
        config: Option<&ConfigFile>,
    ) -> PyResult<Self> {
        let data = data.as_bytes().to_vec();
        let config = config.map(ConfigFile::snapshot).transpose()?.unwrap_or_default();
        py.detach(move || {
            gix::submodule::File::from_bytes(&data, path, &config)
                .map(|inner| Self {
                    inner: Arc::new(Mutex::new(inner)),
                })
                .map_err(to_py)
        })
    }
    fn append_submodule_overrides<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        config: &ConfigFile,
    ) -> PyResult<PyRef<'py, Self>> {
        let config = config.snapshot()?;
        slf.with(py, |inner| {
            inner.append_submodule_overrides(&config).map(|_| ()).map_err(to_py)
        })?;
        Ok(slf)
    }
    fn config(&self, py: Python<'_>) -> PyResult<ConfigFile> {
        self.with(py, |inner| Ok(ConfigFile::from_native(inner.config().clone())))
    }
    fn config_path(&self, py: Python<'_>) -> PyResult<Option<PathBuf>> {
        self.with(py, |inner| Ok(inner.config_path().map(Into::into)))
    }
    fn names(&self, py: Python<'_>) -> PyResult<SubmoduleNames> {
        let inner = self.with(py, |inner| Ok(inner.clone()))?;
        Ok(SubmoduleNames {
            inner: OwnedIter::new("submodule names", None, None, move |_, producer| {
                producer.serve(inner.names().map(|name| Ok(name.to_vec())))
            }),
        })
    }
    fn name_by_path<'py>(
        &self,
        py: Python<'py>,
        relative_path: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let path = bytes(relative_path)?;
        let value = self.with(py, |inner| Ok(inner.name_by_path(path.as_bstr()).map(|v| v.to_vec())))?;
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn path<'py>(&self, py: Python<'py>, name: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyBytes>> {
        let name = bytes(name)?;
        let value = self.with(py, |inner| inner.path(name.as_bstr()).map_err(to_py))?;
        Ok(PyBytes::new(py, &value))
    }
    fn url<'py>(&self, py: Python<'py>, name: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyBytes>> {
        let name = bytes(name)?;
        let value = self.with(py, |inner| {
            inner.url(name.as_bstr()).map(|v| v.to_bstring()).map_err(to_py)
        })?;
        Ok(PyBytes::new(py, &value))
    }
    fn update(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<SubmoduleUpdate>> {
        let name = bytes(name)?;
        self.with(py, |inner| {
            inner
                .update(name.as_bstr())
                .map(|v| v.map(|inner| SubmoduleUpdate { inner }))
                .map_err(to_py)
        })
    }
    fn branch(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<SubmoduleBranch>> {
        let name = bytes(name)?;
        self.with(py, |inner| {
            inner
                .branch(name.as_bstr())
                .map(|v| v.map(|inner| SubmoduleBranch { inner }))
                .map_err(to_py)
        })
    }
    fn fetch_recurse(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<SubmoduleFetchRecurse>> {
        let name = bytes(name)?;
        self.with(py, |inner| {
            inner
                .fetch_recurse(name.as_bstr())
                .map(|v| v.map(|inner| SubmoduleFetchRecurse { inner }))
                .map_err(to_py)
        })
    }
    fn ignore(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<SubmoduleIgnore>> {
        let name = bytes(name)?;
        self.with(py, |inner| {
            inner
                .ignore(name.as_bstr())
                .map(|v| v.map(|inner| SubmoduleIgnore { inner }))
                .map_err(to_py)
        })
    }
    fn shallow(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
        let name = bytes(name)?;
        self.with(py, |inner| inner.shallow(name.as_bstr()).map_err(to_py))
    }
}

type Value = Box<dyn Any + Send>;
type Job = Box<dyn for<'a> FnOnce(&gix::Submodule<'a>) -> PyResult<Value> + Send>;
enum Command {
    Initialize,
    Next,
    Close,
    Call(usize, Job),
}
type Owner = CommandOwner<Command, Value>;
fn downcast<T: Send + 'static>(value: Value) -> PyResult<T> {
    value
        .downcast::<T>()
        .map(|v| *v)
        .map_err(|_| PyValueError::new_err("unexpected native submodule response"))
}
#[pyclass(frozen, module = "gix")]
pub struct SubmoduleIter {
    owner: Arc<Owner>,
    closed: AtomicBool,
}
#[pyclass(frozen, module = "gix")]
pub struct Submodule {
    owner: Arc<Owner>,
    slot: usize,
    name: Vec<u8>,
}
impl Submodule {
    fn call<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&gix::Submodule<'_>) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        downcast(self.owner.call(
            py,
            Command::Call(
                self.slot,
                Box::new(move |submodule| work(submodule).map(|v| Box::new(v) as Value)),
            ),
        )?)
    }
}
#[pymethods]
impl SubmoduleIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Submodule>> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(None);
        }
        let result: Option<(usize, Vec<u8>)> = downcast(self.owner.call(py, Command::Next)?)?;
        if result.is_none() {
            self.closed.store(true, Ordering::Release);
        }
        Ok(result.map(|(slot, name)| Submodule {
            owner: self.owner.clone(),
            slot,
            name,
        }))
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.owner.call(py, Command::Close)?;
        }
        Ok(())
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
impl Submodule {
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }
    fn validated_name<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(
            py,
            &self.call(py, |s| s.validated_name().map(|v| v.to_vec()).map_err(to_py))?,
        ))
    }
    fn path<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.call(py, |s| s.path().map_err(to_py))?))
    }
    fn url<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(
            py,
            &self.call(py, |s| s.url().map(|v| v.to_bstring()).map_err(to_py))?,
        ))
    }
    fn update(&self, py: Python<'_>) -> PyResult<Option<SubmoduleUpdate>> {
        self.call(py, |s| {
            s.update()
                .map(|v| v.map(|inner| SubmoduleUpdate { inner }))
                .map_err(to_py)
        })
    }
    fn branch(&self, py: Python<'_>) -> PyResult<Option<SubmoduleBranch>> {
        self.call(py, |s| {
            s.branch()
                .map(|v| v.map(|inner| SubmoduleBranch { inner }))
                .map_err(to_py)
        })
    }
    fn fetch_recurse(&self, py: Python<'_>) -> PyResult<Option<SubmoduleFetchRecurse>> {
        self.call(py, |s| {
            s.fetch_recurse()
                .map(|v| v.map(|inner| SubmoduleFetchRecurse { inner }))
                .map_err(to_py)
        })
    }
    fn ignore(&self, py: Python<'_>) -> PyResult<Option<SubmoduleIgnore>> {
        self.call(py, |s| {
            s.ignore()
                .map(|v| v.map(|inner| SubmoduleIgnore { inner }))
                .map_err(to_py)
        })
    }
    fn shallow(&self, py: Python<'_>) -> PyResult<Option<bool>> {
        self.call(py, |s| s.shallow().map_err(to_py))
    }
    fn is_active(&self, py: Python<'_>) -> PyResult<bool> {
        self.call(py, |s| s.is_active().map_err(to_py))
    }
    fn index_id(&self, py: Python<'_>) -> PyResult<Option<ObjectId>> {
        self.call(py, |s| {
            s.index_id().map(|v| v.map(|inner| ObjectId { inner })).map_err(to_py)
        })
    }
    fn head_id(&self, py: Python<'_>) -> PyResult<Option<ObjectId>> {
        self.call(py, |s| {
            s.head_id().map(|v| v.map(|inner| ObjectId { inner })).map_err(to_py)
        })
    }
    fn git_dir(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.call(py, |s| s.git_dir().map_err(to_py))
    }
    fn work_dir(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.call(py, |s| s.work_dir().map_err(to_py))
    }
    fn git_dir_try_old_form(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.call(py, |s| s.git_dir_try_old_form().map_err(to_py))
    }
    fn state(&self, py: Python<'_>) -> PyResult<SubmoduleState> {
        self.call(py, |s| s.state().map(|inner| SubmoduleState { inner }).map_err(to_py))
    }
    fn open(&self, py: Python<'_>) -> PyResult<Option<Repository>> {
        self.call(py, |s| s.open().map(|v| v.map(Repository::from_native)).map_err(to_py))
    }
    #[cfg(feature = "status")]
    fn status(
        &self,
        py: Python<'_>,
        ignore: SubmoduleIgnore,
        check_dirty: bool,
    ) -> PyResult<crate::status::SubmoduleStatus> {
        self.call(py, move |s| {
            s.status(ignore.inner, check_dirty)
                .map(|inner| crate::status::SubmoduleStatus { inner })
                .map_err(to_py)
        })
    }
}
#[pymethods]
impl Repository {
    fn open_modules_file(&self, py: Python<'_>) -> PyResult<Option<ModulesFile>> {
        self.handle.run(py, |r| {
            r.open_modules_file()
                .map(|v| {
                    v.map(|inner| ModulesFile {
                        inner: Arc::new(Mutex::new(inner)),
                    })
                })
                .map_err(to_py)
        })
    }
    fn modules(&self, py: Python<'_>) -> PyResult<Option<ModulesFile>> {
        self.handle.run(py, |r| {
            r.modules()
                .map(|v| {
                    v.map(|inner| ModulesFile {
                        inner: Arc::new(Mutex::new((**inner).clone())),
                    })
                })
                .map_err(to_py)
        })
    }
    fn submodules(&self, py: Python<'_>) -> PyResult<Option<SubmoduleIter>> {
        let handle = self.handle.clone();
        let owner = Arc::new(Owner::new("submodules", move |_, commands, producer| {
            handle.with(|repo| {
                let mut iter = repo.submodules().map_err(to_py)?;
                let available = iter.is_some();
                let mut retained = Vec::new();
                producer.serve(std::iter::from_fn(move || {
                    let command = commands.lock().unwrap_or_else(|e| e.into_inner()).take()?;
                    let result = match command {
                        Command::Initialize => Ok(Box::new(available) as Value),
                        Command::Next => {
                            let value = iter.as_mut().and_then(Iterator::next).map(|submodule| {
                                let info = (retained.len(), submodule.name().to_vec());
                                retained.push(submodule);
                                info
                            });
                            Ok(Box::new(value) as Value)
                        }
                        Command::Close => {
                            iter = None;
                            Ok(Box::new(()) as Value)
                        }
                        Command::Call(slot, job) => retained
                            .get(slot)
                            .ok_or_else(|| PyValueError::new_err("submodule is unavailable"))
                            .and_then(job),
                    };
                    Some(Ok(result))
                }))
            })
        }));
        let available: bool = downcast(owner.call(py, Command::Initialize)?)?;
        Ok(available.then_some(SubmoduleIter {
            owner,
            closed: AtomicBool::new(false),
        }))
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SubmoduleIgnore>()?;
    m.add_class::<SubmoduleFetchRecurse>()?;
    m.add_class::<SubmoduleBranch>()?;
    m.add_class::<SubmoduleUpdate>()?;
    m.add_class::<SubmoduleState>()?;
    m.add_class::<ModulesFile>()?;
    m.add_class::<SubmoduleNames>()?;
    m.add_class::<Submodule>()?;
    m.add_class::<SubmoduleIter>()?;
    Ok(())
}
