//! Native remote, connection, fetch and clone builders, owned by Python-free workers.

use crate::{
    error::to_py,
    repository::{OpenOptions, RepoHandle, Repository},
    runtime::{self, CancellationToken, OperationContext, Progress},
    types::{HashKind, ObjectId, bytes},
};
use gix::{
    bstr::{BString, ByteSlice},
    remote::Direction as NativeDirection,
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

#[cfg(feature = "http")]
#[path = "network_tls.rs"]
mod tls;

#[cfg(feature = "worktree-mutation")]
use crate::worktree::CheckoutOutcome;

type Transport = Box<dyn gix::protocol::transport::client::blocking_io::Transport + Send>;
type NativeConnection<'a, 'b, 'c> = gix::remote::Connection<'a, 'b, 'c, Transport>;
type NativeFetch<'a, 'b> = gix::remote::fetch::Prepare<'a, 'b, Transport>;
type RemoteJob = Box<dyn for<'a> FnOnce(&mut gix::Remote<'a>) + Send>;
type ConnectionJob = Box<dyn for<'a, 'b, 'c> FnOnce(&mut Option<NativeConnection<'a, 'b, 'c>>) + Send>;
type FetchJob = Box<dyn for<'a, 'b> FnOnce(&mut Option<NativeFetch<'a, 'b>>) + Send>;
type CloneJob = Box<dyn FnOnce(&mut Option<gix::clone::PrepareFetch>) + Send>;
#[cfg(feature = "worktree-mutation")]
type CheckoutJob = Box<dyn FnOnce(&mut Option<gix::clone::PrepareCheckout>) + Send>;

/// One worker retains the native owner and its borrows; requests contain Rust values only.
/// Parent builders remain busy while a borrowed child connection is alive.
struct Owner<J> {
    sender: Sender<J>,
    busy: Arc<AtomicBool>,
}
impl<J> Clone for Owner<J> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            busy: self.busy.clone(),
        }
    }
}
struct Release(Arc<AtomicBool>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Call<T> {
    context: OperationContext,
    result: Sender<PyResult<T>>,
    release: std::cell::RefCell<Option<Release>>,
}
impl<T> Call<T> {
    fn answer(&self, value: PyResult<T>) {
        self.release.borrow_mut().take();
        let _ = self.result.send(value);
    }
    fn enter(&self, value: PyResult<T>) {
        let _ = self.result.send(value);
    }
}

fn channel<J>() -> (Owner<J>, Receiver<J>) {
    let (sender, receiver) = mpsc::channel();
    (
        Owner {
            sender,
            busy: Arc::new(AtomicBool::new(false)),
        },
        receiver,
    )
}
fn disconnected() -> PyErr {
    PyRuntimeError::new_err("native network owner is closed")
}
fn consumed() -> PyErr {
    PyRuntimeError::new_err("native builder has already been consumed")
}

impl<J: Send + 'static> Owner<J> {
    fn request<T: Send + 'static>(
        &self,
        py: Python<'_>,
        name: &'static str,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
        job: impl FnOnce(Call<T>) -> J + Send + 'static,
    ) -> PyResult<T> {
        let owner = self.clone();
        runtime::run(py, name, progress, cancel, move |context| {
            if owner
                .busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(PyRuntimeError::new_err(
                    "native builder is executing or borrowed by a connection",
                ));
            }
            let (result, receiver) = mpsc::channel();
            owner
                .sender
                .send(job(Call {
                    context,
                    result,
                    release: std::cell::RefCell::new(Some(Release(owner.busy.clone()))),
                }))
                .map_err(|_| disconnected())?;
            receiver.recv().map_err(|_| disconnected())?
        })?
    }
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Direction {
    pub(crate) inner: NativeDirection,
}
#[pymethods]
impl Direction {
    #[classattr]
    #[pyo3(name = "Fetch")]
    fn fetch() -> Self {
        Self {
            inner: NativeDirection::Fetch,
        }
    }
    #[classattr]
    #[pyo3(name = "Push")]
    fn push() -> Self {
        Self {
            inner: NativeDirection::Push,
        }
    }
    fn as_str(&self) -> &'static str {
        self.inner.as_str()
    }
    fn __repr__(&self) -> String {
        format!("Direction.{:?}", self.inner)
    }
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Tags {
    inner: gix::remote::fetch::Tags,
}
#[pymethods]
impl Tags {
    #[classattr]
    #[pyo3(name = "All")]
    fn all() -> Self {
        Self {
            inner: gix::remote::fetch::Tags::All,
        }
    }
    #[classattr]
    #[pyo3(name = "Included")]
    fn included() -> Self {
        Self {
            inner: gix::remote::fetch::Tags::Included,
        }
    }
    #[classattr]
    #[pyo3(name = "None_")]
    fn none() -> Self {
        Self {
            inner: gix::remote::fetch::Tags::None,
        }
    }
    fn __repr__(&self) -> String {
        format!("Tags.{:?}", self.inner)
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Shallow {
    inner: gix::remote::fetch::Shallow,
}
#[pymethods]
impl Shallow {
    #[classattr]
    #[pyo3(name = "NoChange")]
    fn no_change() -> Self {
        Self {
            inner: Default::default(),
        }
    }
    #[staticmethod]
    #[pyo3(name = "DepthAtRemote")]
    fn depth_at_remote(depth: u32) -> PyResult<Self> {
        Ok(Self {
            inner: gix::remote::fetch::Shallow::DepthAtRemote(
                depth
                    .try_into()
                    .map_err(|_| PyValueError::new_err("depth must be nonzero"))?,
            ),
        })
    }
    #[staticmethod]
    #[pyo3(name = "Deepen")]
    fn deepen(depth: u32) -> Self {
        Self {
            inner: gix::remote::fetch::Shallow::Deepen(depth),
        }
    }
    #[staticmethod]
    #[pyo3(name = "Since")]
    fn since(seconds: i64) -> Self {
        Self {
            inner: gix::remote::fetch::Shallow::Since {
                cutoff: gix::date::Time { seconds, offset: 0 },
            },
        }
    }
    #[staticmethod]
    fn undo() -> Self {
        Self {
            inner: gix::remote::fetch::Shallow::undo(),
        }
    }
    fn __repr__(&self) -> String {
        format!("Shallow({:?})", self.inner)
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RefSpec {
    inner: gix::refspec::RefSpec,
}
#[pymethods]
impl RefSpec {
    #[new]
    fn new(spec: &Bound<'_, PyAny>, direction: Direction) -> PyResult<Self> {
        let spec = bytes(spec)?;
        Ok(Self {
            inner: gix::refspec::parse(
                spec.as_bstr(),
                match direction.inner {
                    NativeDirection::Fetch => gix::refspec::parse::Operation::Fetch,
                    NativeDirection::Push => gix::refspec::parse::Operation::Push,
                },
            )
            .map_err(to_py)?
            .to_owned(),
        })
    }
    fn source<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner.to_ref().source().map(|value| PyBytes::new(py, value))
    }
    fn destination<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner.to_ref().destination().map(|value| PyBytes::new(py, value))
    }
    fn allow_non_fast_forward(&self) -> bool {
        self.inner.allow_non_fast_forward()
    }
    fn to_bstring<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        self.__bytes__(py)
    }
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.to_ref().to_bstring().as_ref())
    }
    fn __repr__(&self) -> String {
        format!("RefSpec({:?})", self.inner.to_ref().to_bstring())
    }
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct RefMapOptions {
    inner: gix::remote::ref_map::Options,
}
#[pymethods]
impl RefMapOptions {
    #[new]
    #[pyo3(signature = (*, prefix_from_spec_as_filter_on_remote=true, handshake_parameters=None, extra_refspecs=None))]
    fn new(
        prefix_from_spec_as_filter_on_remote: bool,
        handshake_parameters: Option<Vec<(String, Option<String>)>>,
        extra_refspecs: Option<Vec<RefSpec>>,
    ) -> Self {
        Self {
            inner: gix::remote::ref_map::Options {
                prefix_from_spec_as_filter_on_remote,
                handshake_parameters: handshake_parameters.unwrap_or_default(),
                extra_refspecs: extra_refspecs
                    .unwrap_or_default()
                    .into_iter()
                    .map(|v| v.inner)
                    .collect(),
            },
        }
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Remote {
    owner: Owner<RemoteJob>,
    handle: RepoHandle,
}

pub(crate) fn make_remote(
    py: Python<'_>,
    handle: RepoHandle,
    create: impl for<'a> FnOnce(&'a gix::Repository) -> PyResult<Option<gix::Remote<'a>>> + Send + 'static,
) -> PyResult<Option<Remote>> {
    runtime::run(py, "remote", None, None, move |_| {
        let (owner, requests) = channel::<RemoteJob>();
        let (ready, result) = mpsc::channel();
        let worker_handle = handle.clone();
        thread::Builder::new()
            .name("gix-remote-owner".into())
            .spawn(move || {
                let result = worker_handle.with(|repo| {
                    let Some(mut native) = create(repo)? else {
                        let _ = ready.send(Ok(false));
                        return Ok(());
                    };
                    if ready.send(Ok(true)).is_ok() {
                        for request in requests {
                            request(&mut native);
                        }
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    let _ = ready.send(Err(error));
                }
            })
            .map_err(to_py)?;
        result
            .recv()
            .map_err(|_| disconnected())?
            .map(|found| found.then_some(Remote { owner, handle }))
    })?
}

#[pymethods]
impl Repository {
    fn remote_names<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyBytes>>> {
        let names = self.handle.run(py, |repo| Ok(repo.remote_names()))?;
        Ok(names.into_iter().map(|name| PyBytes::new(py, &name)).collect())
    }
    fn remote_default_name<'py>(&self, py: Python<'py>, direction: Direction) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let name = self
            .handle
            .run(py, move |repo| Ok(repo.remote_default_name(direction.inner)))?;
        Ok(name.map(|name| PyBytes::new(py, &name)))
    }
    fn try_find_remote_without_url_rewrite(
        &self,
        py: Python<'_>,
        name_or_url: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Remote>> {
        let name = bytes(name_or_url)?;
        make_remote(py, self.handle.clone(), move |repo| {
            repo.try_find_remote_without_url_rewrite(name.as_bstr())
                .transpose()
                .map_err(to_py)
        })
    }
    fn remote_at(&self, py: Python<'_>, url: &Bound<'_, PyAny>) -> PyResult<Remote> {
        let url = bytes(url)?;
        make_remote(py, self.handle.clone(), move |repo| {
            repo.remote_at(url.as_bstr()).map(Some).map_err(to_py)
        })?
        .ok_or_else(disconnected)
    }
    fn remote_at_without_url_rewrite(&self, py: Python<'_>, url: &Bound<'_, PyAny>) -> PyResult<Remote> {
        let url = bytes(url)?;
        make_remote(py, self.handle.clone(), move |repo| {
            repo.remote_at_without_url_rewrite(url.as_bstr())
                .map(Some)
                .map_err(to_py)
        })?
        .ok_or_else(disconnected)
    }
    fn find_remote(&self, py: Python<'_>, name_or_url: &Bound<'_, PyAny>) -> PyResult<Remote> {
        let name = bytes(name_or_url)?;
        make_remote(py, self.handle.clone(), move |repo| {
            repo.find_remote(name.as_bstr()).map(Some).map_err(to_py)
        })?
        .ok_or_else(disconnected)
    }
    fn try_find_remote(&self, py: Python<'_>, name_or_url: &Bound<'_, PyAny>) -> PyResult<Option<Remote>> {
        let name = bytes(name_or_url)?;
        make_remote(py, self.handle.clone(), move |repo| {
            repo.try_find_remote(name.as_bstr()).transpose().map_err(to_py)
        })
    }
    fn find_default_remote(&self, py: Python<'_>, direction: Direction) -> PyResult<Option<Remote>> {
        make_remote(py, self.handle.clone(), move |repo| {
            repo.find_default_remote(direction.inner).transpose().map_err(to_py)
        })
    }
    #[pyo3(signature = (name_or_url=None))]
    fn find_fetch_remote(&self, py: Python<'_>, name_or_url: Option<&Bound<'_, PyAny>>) -> PyResult<Remote> {
        let name = name_or_url.map(bytes).transpose()?;
        make_remote(py, self.handle.clone(), move |repo| {
            repo.find_fetch_remote(name.as_deref().map(ByteSlice::as_bstr))
                .map(Some)
                .map_err(to_py)
        })?
        .ok_or_else(disconnected)
    }
}

#[pymethods]
impl Remote {
    fn with_url<'py>(slf: PyRef<'py, Self>, py: Python<'py>, url: &Bound<'_, PyAny>) -> PyResult<PyRef<'py, Self>> {
        let url = bytes(url)?;
        slf.owner.request(py, "configure remote URL", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(
                    remote
                        .clone()
                        .with_url(url.as_bstr())
                        .map(|value| *remote = value)
                        .map_err(to_py),
                );
            })
        })?;
        Ok(slf)
    }
    fn with_url_without_url_rewrite<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        url: &Bound<'_, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        let url = bytes(url)?;
        slf.owner.request(py, "configure remote URL", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(
                    remote
                        .clone()
                        .with_url_without_url_rewrite(url.as_bstr())
                        .map(|value| *remote = value)
                        .map_err(to_py),
                );
            })
        })?;
        Ok(slf)
    }
    fn with_push_url<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        url: &Bound<'_, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        let url = bytes(url)?;
        slf.owner.request(py, "configure remote URL", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(
                    remote
                        .clone()
                        .with_push_url(url.as_bstr())
                        .map(|value| *remote = value)
                        .map_err(to_py),
                );
            })
        })?;
        Ok(slf)
    }
    fn with_push_url_without_url_rewrite<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        url: &Bound<'_, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        let url = bytes(url)?;
        slf.owner.request(py, "configure remote URL", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(
                    remote
                        .clone()
                        .with_push_url_without_url_rewrite(url.as_bstr())
                        .map(|value| *remote = value)
                        .map_err(to_py),
                );
            })
        })?;
        Ok(slf)
    }
    fn repo(&self) -> Repository {
        Repository {
            handle: self.handle.clone(),
        }
    }
    fn name<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = self.owner.request(py, "remote name", None, None, |call| {
            Box::new(move |remote| call.answer(Ok(remote.name().map(|n| n.as_bstr().to_vec()))))
        })?;
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn url<'py>(&self, py: Python<'py>, direction: Direction) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let value = self.owner.request(py, "remote URL", None, None, move |call| {
            Box::new(move |remote| call.answer(Ok(remote.url(direction.inner).map(|u| u.to_bstring().to_vec()))))
        })?;
        Ok(value.map(|v| PyBytes::new(py, &v)))
    }
    fn refspecs(&self, py: Python<'_>, direction: Direction) -> PyResult<Vec<RefSpec>> {
        self.owner.request(py, "remote refspecs", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(Ok(remote
                    .refspecs(direction.inner)
                    .iter()
                    .cloned()
                    .map(|inner| RefSpec { inner })
                    .collect()))
            })
        })
    }
    fn fetch_tags(&self, py: Python<'_>) -> PyResult<Tags> {
        self.owner.request(py, "remote tags", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(Ok(Tags {
                    inner: remote.fetch_tags(),
                }))
            })
        })
    }
    fn with_fetch_tags<'py>(slf: PyRef<'py, Self>, py: Python<'py>, tags: Tags) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure remote", None, None, move |call| {
            Box::new(move |remote| {
                *remote = remote.clone().with_fetch_tags(tags.inner);
                call.answer(Ok(()));
            })
        })?;
        Ok(slf)
    }
    fn with_refspecs<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        specs: Vec<RefSpec>,
        direction: Direction,
    ) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure remote", None, None, move |call| {
            Box::new(move |remote| {
                let result = remote
                    .clone()
                    .with_refspecs(specs.iter().map(|s| s.inner.to_ref().to_bstring()), direction.inner)
                    .map(|value| *remote = value)
                    .map_err(to_py);
                call.answer(result);
            })
        })?;
        Ok(slf)
    }
    fn replace_refspecs(&self, py: Python<'_>, specs: Vec<RefSpec>, direction: Direction) -> PyResult<()> {
        self.owner.request(py, "configure remote", None, None, move |call| {
            Box::new(move |remote| {
                call.answer(
                    remote
                        .replace_refspecs(specs.iter().map(|s| s.inner.to_ref().to_bstring()), direction.inner)
                        .map_err(to_py),
                )
            })
        })
    }
    fn rewrite_urls(&self, py: Python<'_>) -> PyResult<()> {
        self.owner.request(py, "rewrite URLs", None, None, |call| {
            Box::new(move |remote| call.answer(remote.rewrite_urls().map(|_| ()).map_err(to_py)))
        })
    }
    #[pyo3(signature = (direction, *, progress=None, cancel=None))]
    fn connect(
        &self,
        py: Python<'_>,
        direction: Direction,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<Connection> {
        self.owner
            .request(py, "connect", progress.as_ref(), cancel.as_ref(), move |call| {
                Box::new(move |remote| {
                    let result = remote.connect(direction.inner).map_err(to_py);
                    #[cfg(feature = "http")]
                    let result = result.and_then(|mut connection| {
                        tls::configure(&mut connection).map_err(to_py)?;
                        Ok(connection)
                    });
                    match result {
                        Err(error) => call.answer(Err(error)),
                        Ok(connection) => {
                            let (owner, requests) = channel::<ConnectionJob>();
                            call.enter(Ok(Connection { owner }));
                            let mut connection = Some(connection);
                            for request in requests {
                                request(&mut connection);
                                if connection.is_none() {
                                    break;
                                }
                            }
                        }
                    }
                })
            })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Connection {
    owner: Owner<ConnectionJob>,
}
#[pymethods]
impl Connection {
    #[pyo3(signature = (*, options=None, progress=None, cancel=None))]
    fn ref_map(
        &self,
        py: Python<'_>,
        options: Option<RefMapOptions>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<(RefMap, Handshake)> {
        self.owner.request(
            py,
            "remote ref map",
            progress.as_ref(),
            cancel.as_ref(),
            move |mut call| {
                Box::new(move |connection| {
                    let result = connection.take().ok_or_else(consumed).and_then(|connection| {
                        connection
                            .ref_map(&mut call.context.progress, options.unwrap_or_default().inner)
                            .map(|(map, handshake)| {
                                (
                                    RefMap { inner: Arc::new(map) },
                                    Handshake {
                                        inner: Arc::new(handshake),
                                    },
                                )
                            })
                            .map_err(to_py)
                    });
                    call.answer(result);
                })
            },
        )
    }
    #[pyo3(signature = (*, options=None, progress=None, cancel=None))]
    fn prepare_fetch(
        &self,
        py: Python<'_>,
        options: Option<RefMapOptions>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<PrepareFetch> {
        self.owner.request(
            py,
            "prepare fetch",
            progress.as_ref(),
            cancel.as_ref(),
            move |mut call| {
                Box::new(move |connection| {
                    let result = connection.take().ok_or_else(consumed).and_then(|connection| {
                        connection
                            .prepare_fetch(&mut call.context.progress, options.unwrap_or_default().inner)
                            .map_err(to_py)
                    });
                    match result {
                        Err(error) => call.answer(Err(error)),
                        Ok(fetch) => {
                            let (owner, requests) = channel::<FetchJob>();
                            call.enter(Ok(PrepareFetch { owner }));
                            let mut fetch = Some(fetch);
                            for request in requests {
                                request(&mut fetch);
                                if fetch.is_none() {
                                    break;
                                }
                            }
                        }
                    }
                })
            },
        )
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct PrepareFetch {
    owner: Owner<FetchJob>,
}
#[pymethods]
impl PrepareFetch {
    fn ref_map(&self, py: Python<'_>) -> PyResult<RefMap> {
        self.owner.request(py, "fetch ref map", None, None, |call| {
            Box::new(move |fetch| {
                call.answer(
                    fetch
                        .as_ref()
                        .map(|f| RefMap {
                            inner: Arc::new(f.ref_map().clone()),
                        })
                        .ok_or_else(consumed),
                )
            })
        })
    }
    fn with_dry_run<'py>(slf: PyRef<'py, Self>, py: Python<'py>, enabled: bool) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure fetch", None, None, move |call| {
            Box::new(move |fetch| {
                call.answer(
                    fetch
                        .take()
                        .ok_or_else(consumed)
                        .map(|value| *fetch = Some(value.with_dry_run(enabled))),
                )
            })
        })?;
        Ok(slf)
    }
    fn with_write_packed_refs_only<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        enabled: bool,
    ) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure fetch", None, None, move |call| {
            Box::new(move |fetch| {
                call.answer(
                    fetch
                        .take()
                        .ok_or_else(consumed)
                        .map(|value| *fetch = Some(value.with_write_packed_refs_only(enabled))),
                )
            })
        })?;
        Ok(slf)
    }
    fn with_shallow<'py>(slf: PyRef<'py, Self>, py: Python<'py>, shallow: Shallow) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure fetch", None, None, move |call| {
            Box::new(move |fetch| {
                call.answer(
                    fetch
                        .take()
                        .ok_or_else(consumed)
                        .map(|value| *fetch = Some(value.with_shallow(shallow.inner))),
                )
            })
        })?;
        Ok(slf)
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn receive(
        &self,
        py: Python<'_>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<FetchOutcome> {
        self.owner
            .request(py, "receive fetch", progress.as_ref(), cancel.as_ref(), |mut call| {
                Box::new(move |fetch| {
                    let result = fetch.take().ok_or_else(consumed).and_then(|fetch| {
                        fetch
                            .receive(&mut call.context.progress, &call.context.interrupt)
                            .map(|inner| FetchOutcome { inner: Arc::new(inner) })
                            .map_err(to_py)
                    });
                    call.answer(result);
                })
            })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct PrepareClone {
    owner: Owner<CloneJob>,
}

#[pyfunction]
#[pyo3(signature = (url, path, *, options=None))]
fn prepare_clone(
    py: Python<'_>,
    url: &Bound<'_, PyAny>,
    path: PathBuf,
    options: Option<OpenOptions>,
) -> PyResult<PrepareClone> {
    prepare_clone_inner(py, bytes(url)?, path, options, gix::create::Kind::WithWorktree)
}
#[pyfunction]
#[pyo3(signature = (url, path, *, options=None))]
fn prepare_clone_bare(
    py: Python<'_>,
    url: &Bound<'_, PyAny>,
    path: PathBuf,
    options: Option<OpenOptions>,
) -> PyResult<PrepareClone> {
    prepare_clone_inner(py, bytes(url)?, path, options, gix::create::Kind::Bare)
}

fn prepare_clone_inner(
    py: Python<'_>,
    url: Vec<u8>,
    path: PathBuf,
    options: Option<OpenOptions>,
    kind: gix::create::Kind,
) -> PyResult<PrepareClone> {
    runtime::run(py, "prepare clone", None, None, move |_| {
        let (owner, requests) = channel::<CloneJob>();
        let (ready, result) = mpsc::channel();
        thread::Builder::new()
            .name("gix-clone-owner".into())
            .spawn(move || {
                let native = gix::clone::PrepareFetch::new(
                    url.as_bstr(),
                    path,
                    kind,
                    Default::default(),
                    options.unwrap_or_default().inner,
                )
                .map_err(to_py);
                match native {
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                    Ok(native) => {
                        #[cfg(feature = "http")]
                        let native = native.configure_connection(tls::configure);
                        if ready.send(Ok(())).is_ok() {
                            let mut native = Some(native);
                            for request in requests {
                                request(&mut native);
                                if native.is_none() {
                                    break;
                                }
                            }
                        }
                    }
                }
            })
            .map_err(to_py)?;
        result.recv().map_err(|_| disconnected())??;
        Ok(PrepareClone { owner })
    })?
}

#[pymethods]
impl PrepareClone {
    #[pyo3(signature = (name=None))]
    fn with_ref_name<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        name: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRef<'py, Self>> {
        let name = name.map(bytes).transpose()?;
        if let Some(name) = &name {
            <&gix::refs::PartialNameRef>::try_from(name.as_bstr()).map_err(to_py)?;
        }
        slf.owner.request(py, "configure clone", None, None, move |call| {
            Box::new(move |clone| {
                call.answer(clone.take().ok_or_else(consumed).and_then(|value| {
                    value
                        .with_ref_name(name.as_deref().map(ByteSlice::as_bstr))
                        .map(|value| *clone = Some(value))
                        .map_err(to_py)
                }));
            })
        })?;
        Ok(slf)
    }
    #[pyo3(signature = (revision=None))]
    fn with_revision<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        revision: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRef<'py, Self>> {
        let revision = revision.map(bytes).transpose()?.map(BString::from);
        if let Some(revision) = &revision {
            // Mirror the native validation before taking its cleanup-owning builder.
            // A consuming native error would otherwise remove the clone destination.
            let spec = gix::refspec::parse(revision.as_ref(), gix::refspec::parse::Operation::Fetch).map_err(to_py)?;
            let valid = spec.source().is_some_and(|source| {
                let full_ref = source.starts_with(b"refs/") && source.find_byteset(b"*?[]\\").is_none();
                revision.as_bstr() == source
                    && spec.destination().is_none()
                    && (source == "HEAD" || full_ref || gix::ObjectId::from_hex(source).is_ok())
            });
            if !valid {
                return Err(to_py(gix::clone::with_revision::Error::Invalid {
                    revision: revision.clone(),
                }));
            }
        }
        slf.owner.request(py, "configure clone", None, None, move |call| {
            Box::new(move |clone| {
                call.answer(clone.take().ok_or_else(consumed).and_then(|value| {
                    value
                        .with_revision(revision)
                        .map(|value| *clone = Some(value))
                        .map_err(to_py)
                }));
            })
        })?;
        Ok(slf)
    }
    fn with_remote_name<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        name: &Bound<'_, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        let name: BString = bytes(name)?.into();
        // Validate before consuming the native builder, preserving its cleanup/retry state on invalid input.
        gix::remote::name::validated(name.clone()).map_err(to_py)?;
        slf.owner.request(py, "configure clone", None, None, move |call| {
            Box::new(move |clone| {
                call.answer(
                    clone
                        .take()
                        .ok_or_else(consumed)
                        .and_then(|value| value.with_remote_name(name).map(|v| *clone = Some(v)).map_err(to_py)),
                )
            })
        })?;
        Ok(slf)
    }
    fn with_shallow<'py>(slf: PyRef<'py, Self>, py: Python<'py>, shallow: Shallow) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure clone", None, None, move |call| {
            Box::new(move |clone| {
                call.answer(
                    clone
                        .take()
                        .ok_or_else(consumed)
                        .map(|value| *clone = Some(value.with_shallow(shallow.inner))),
                )
            })
        })?;
        Ok(slf)
    }
    fn with_in_memory_config_overrides<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        values: Vec<String>,
    ) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure clone", None, None, move |call| {
            Box::new(move |clone| {
                call.answer(
                    clone
                        .take()
                        .ok_or_else(consumed)
                        .map(|value| *clone = Some(value.with_in_memory_config_overrides(values))),
                )
            })
        })?;
        Ok(slf)
    }
    fn with_fetch_options<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        options: RefMapOptions,
    ) -> PyResult<PyRef<'py, Self>> {
        slf.owner.request(py, "configure clone", None, None, move |call| {
            Box::new(move |clone| {
                call.answer(
                    clone
                        .take()
                        .ok_or_else(consumed)
                        .map(|value| *clone = Some(value.with_fetch_options(options.inner))),
                )
            })
        })?;
        Ok(slf)
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn fetch_only(
        &self,
        py: Python<'_>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<(Repository, FetchOutcome)> {
        self.owner
            .request(py, "clone fetch", progress.as_ref(), cancel.as_ref(), |mut call| {
                Box::new(move |clone| {
                    let result = clone.as_mut().ok_or_else(consumed).and_then(|clone| {
                        clone
                            .fetch_only(&mut call.context.progress, &call.context.interrupt)
                            .map_err(to_py)
                    });
                    if result.is_ok() {
                        clone.take();
                    }
                    call.answer(result.map(|(repo, outcome)| {
                        (
                            Repository::from_native(repo),
                            FetchOutcome {
                                inner: Arc::new(outcome),
                            },
                        )
                    }));
                })
            })
    }
    fn persist(&self, py: Python<'_>) -> PyResult<Repository> {
        self.owner.request(py, "persist clone", None, None, |call| {
            Box::new(move |clone| {
                call.answer(
                    clone
                        .take()
                        .map(|clone| Repository::from_native(clone.persist()))
                        .ok_or_else(consumed),
                )
            })
        })
    }
    #[cfg(feature = "worktree-mutation")]
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn fetch_then_checkout(
        &self,
        py: Python<'_>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<(PrepareCheckout, FetchOutcome)> {
        self.owner
            .request(py, "clone fetch", progress.as_ref(), cancel.as_ref(), |mut call| {
                Box::new(move |clone| {
                    let result = clone.as_mut().ok_or_else(consumed).and_then(|clone| {
                        clone
                            .fetch_then_checkout(&mut call.context.progress, &call.context.interrupt)
                            .map_err(to_py)
                    });
                    match result {
                        Err(error) => call.answer(Err(error)),
                        Ok((checkout, outcome)) => {
                            clone.take();
                            let (owner, requests) = channel::<CheckoutJob>();
                            call.enter(Ok((
                                PrepareCheckout { owner },
                                FetchOutcome {
                                    inner: Arc::new(outcome),
                                },
                            )));
                            let mut checkout = Some(checkout);
                            for request in requests {
                                request(&mut checkout);
                                if checkout.is_none() {
                                    break;
                                }
                            }
                        }
                    }
                })
            })
    }
}

#[cfg(feature = "worktree-mutation")]
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct PrepareCheckout {
    owner: Owner<CheckoutJob>,
}
#[cfg(feature = "worktree-mutation")]
#[pymethods]
impl PrepareCheckout {
    fn repo(&self, py: Python<'_>) -> PyResult<Repository> {
        self.owner.request(py, "checkout repository", None, None, |call| {
            Box::new(move |checkout| {
                call.answer(
                    checkout
                        .as_ref()
                        .map(|c| Repository::from_native(c.repo().clone()))
                        .ok_or_else(consumed),
                )
            })
        })
    }
    fn persist(&self, py: Python<'_>) -> PyResult<Repository> {
        self.owner.request(py, "persist checkout", None, None, |call| {
            Box::new(move |checkout| {
                call.answer(
                    checkout
                        .take()
                        .map(|c| Repository::from_native(c.persist()))
                        .ok_or_else(consumed),
                )
            })
        })
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn main_worktree(
        &self,
        py: Python<'_>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<(Repository, CheckoutOutcome)> {
        self.owner
            .request(py, "checkout", progress.as_ref(), cancel.as_ref(), |mut call| {
                Box::new(move |checkout| {
                    let result = checkout.as_mut().ok_or_else(consumed).and_then(|c| {
                        c.main_worktree(&mut call.context.progress, &call.context.interrupt)
                            .map_err(to_py)
                    });
                    if result.is_ok() {
                        checkout.take();
                    }
                    call.answer(result.map(|(repo, inner)| {
                        (
                            Repository::from_native(repo),
                            CheckoutOutcome { inner: Arc::new(inner) },
                        )
                    }));
                })
            })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RefMap {
    inner: Arc<gix::remote::fetch::RefMap>,
}
#[pymethods]
impl RefMap {
    #[getter]
    fn object_hash(&self) -> HashKind {
        HashKind {
            inner: self.inner.object_hash,
        }
    }
    #[getter]
    fn refspecs(&self) -> Vec<RefSpec> {
        self.inner
            .refspecs
            .iter()
            .cloned()
            .map(|inner| RefSpec { inner })
            .collect()
    }
    #[getter]
    fn extra_refspecs(&self) -> Vec<RefSpec> {
        self.inner
            .extra_refspecs
            .iter()
            .cloned()
            .map(|inner| RefSpec { inner })
            .collect()
    }
    #[getter]
    fn remote_refs(&self) -> Vec<RemoteRef> {
        self.inner
            .remote_refs
            .iter()
            .cloned()
            .map(|inner| RemoteRef { inner })
            .collect()
    }
    fn is_missing_required_mapping(&self) -> bool {
        self.inner.is_missing_required_mapping()
    }
    fn __repr__(&self) -> String {
        format!(
            "RefMap({} mappings, {} remote refs)",
            self.inner.mappings.len(),
            self.inner.remote_refs.len()
        )
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RemoteRef {
    inner: gix::protocol::handshake::Ref,
}
#[pymethods]
impl RemoteRef {
    fn unpack<'py>(&self, py: Python<'py>) -> (Bound<'py, PyBytes>, Option<ObjectId>, Option<ObjectId>) {
        let (name, target, peeled) = self.inner.unpack();
        (
            PyBytes::new(py, name),
            target.map(|id| ObjectId { inner: id.to_owned() }),
            peeled.map(|id| ObjectId { inner: id.to_owned() }),
        )
    }
    fn __repr__(&self) -> String {
        format!("RemoteRef({:?})", self.inner)
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Handshake {
    inner: Arc<gix::protocol::Handshake>,
}
#[pymethods]
impl Handshake {
    #[getter]
    fn server_protocol_version(&self) -> String {
        format!("{:?}", self.inner.server_protocol_version)
    }
    #[getter]
    fn refs(&self) -> Option<Vec<RemoteRef>> {
        self.inner
            .refs
            .as_ref()
            .map(|refs| refs.iter().cloned().map(|inner| RemoteRef { inner }).collect())
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct FetchOutcome {
    inner: Arc<gix::remote::fetch::Outcome>,
}
#[pymethods]
impl FetchOutcome {
    #[getter]
    fn ref_map(&self) -> RefMap {
        RefMap {
            inner: Arc::new(self.inner.ref_map.clone()),
        }
    }
    #[getter]
    fn handshake(&self) -> Handshake {
        Handshake {
            inner: Arc::new(self.inner.handshake.clone()),
        }
    }
    #[getter]
    fn status(&self) -> String {
        match &self.inner.status {
            gix::remote::fetch::Status::NoPackReceived { .. } => "NoPackReceived",
            gix::remote::fetch::Status::Change { .. } => "Change",
        }
        .into()
    }
    fn __repr__(&self) -> String {
        format!("FetchOutcome(status={})", self.status())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Direction>()?;
    m.add_class::<Tags>()?;
    m.add_class::<Shallow>()?;
    m.add_class::<RefSpec>()?;
    m.add_class::<RefMapOptions>()?;
    m.add_class::<Remote>()?;
    m.add_class::<Connection>()?;
    m.add_class::<PrepareFetch>()?;
    m.add_class::<PrepareClone>()?;
    m.add_class::<RefMap>()?;
    m.add_class::<RemoteRef>()?;
    m.add_class::<Handshake>()?;
    m.add_class::<FetchOutcome>()?;
    #[cfg(feature = "worktree-mutation")]
    {
        m.add_class::<PrepareCheckout>()?;
    }
    m.add_function(wrap_pyfunction!(prepare_clone, m)?)?;
    m.add_function(wrap_pyfunction!(prepare_clone_bare, m)?)?;
    Ok(())
}
