//! Native blob merge resources and a worker-owned borrowed preparation.
use crate::{
    diff_options::algorithm,
    error::to_py,
    merge::MergeLabels,
    repository::{RepoHandle, Repository},
    runtime::{self, CancellationToken, CommandOwner, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};
use gix::{
    bstr::ByteSlice,
    merge::blob::{self, platform::builtin_merge::Pick},
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};
use std::{
    any::Any,
    num::NonZeroU8,
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn resource_kind(value: &str) -> PyResult<blob::ResourceKind> {
    match value {
        "current" => Ok(blob::ResourceKind::CurrentOrOurs),
        "ancestor" => Ok(blob::ResourceKind::CommonAncestorOrBase),
        "other" => Ok(blob::ResourceKind::OtherOrTheirs),
        _ => Err(PyValueError::new_err(
            "resource kind must be current, ancestor, or other",
        )),
    }
}
fn pick(value: &str) -> PyResult<Pick> {
    match value {
        "Ancestor" => Ok(Pick::Ancestor),
        "Ours" => Ok(Pick::Ours),
        "Theirs" => Ok(Pick::Theirs),
        "Buffer" => Ok(Pick::Buffer),
        _ => Err(PyValueError::new_err("pick must be Ancestor, Ours, Theirs, or Buffer")),
    }
}
fn driver(value: &str) -> PyResult<blob::BuiltinDriver> {
    blob::BuiltinDriver::by_name(value)
        .ok_or_else(|| PyValueError::new_err("builtin driver must be text, binary, or union"))
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct BlobMergeOptions {
    inner: blob::platform::merge::Options,
}
#[pymethods]
impl BlobMergeOptions {
    #[new]
    #[pyo3(signature=(*,is_virtual_ancestor=false,resolve_binary_with=None,diff_algorithm="myers",conflict="keep",conflict_style="merge",marker_size=7))]
    fn new(
        is_virtual_ancestor: bool,
        resolve_binary_with: Option<&str>,
        diff_algorithm: &str,
        conflict: &str,
        conflict_style: &str,
        marker_size: u8,
    ) -> PyResult<Self> {
        use blob::builtin_driver::{
            binary::ResolveWith,
            text::{Conflict, ConflictStyle},
        };
        let resolve_binary_with = match resolve_binary_with {
            None => None,
            Some("ancestor") => Some(ResolveWith::Ancestor),
            Some("ours") => Some(ResolveWith::Ours),
            Some("theirs") => Some(ResolveWith::Theirs),
            _ => {
                return Err(PyValueError::new_err(
                    "binary resolution must be ancestor, ours, theirs, or None",
                ));
            }
        };
        let style = match conflict_style {
            "merge" => ConflictStyle::Merge,
            "diff3" => ConflictStyle::Diff3,
            "zdiff3" => ConflictStyle::ZealousDiff3,
            _ => return Err(PyValueError::new_err("conflict style must be merge, diff3, or zdiff3")),
        };
        let marker_size = NonZeroU8::new(marker_size)
            .ok_or_else(|| PyValueError::new_err("marker size must be between 1 and 255"))?;
        let conflict = match conflict {
            "keep" => Conflict::Keep { style, marker_size },
            "ours" => Conflict::ResolveWithOurs,
            "theirs" => Conflict::ResolveWithTheirs,
            "union" => Conflict::ResolveWithUnion,
            _ => {
                return Err(PyValueError::new_err(
                    "text conflict resolution must be keep, ours, theirs, or union",
                ));
            }
        };
        Ok(Self {
            inner: blob::platform::merge::Options {
                is_virtual_ancestor,
                resolve_binary_with,
                text: blob::builtin_driver::text::Options {
                    diff_algorithm: algorithm(diff_algorithm)?,
                    conflict,
                },
            },
        })
    }
    #[getter]
    fn is_virtual_ancestor(&self) -> bool {
        self.inner.is_virtual_ancestor
    }
    #[getter]
    fn resolve_binary_with(&self) -> Option<&'static str> {
        use blob::builtin_driver::binary::ResolveWith;
        self.inner.resolve_binary_with.map(|v| match v {
            ResolveWith::Ancestor => "ancestor",
            ResolveWith::Ours => "ours",
            ResolveWith::Theirs => "theirs",
        })
    }
    #[getter]
    fn diff_algorithm(&self) -> &'static str {
        use gix::diff::blob::Algorithm;
        match self.inner.text.diff_algorithm {
            Algorithm::Histogram => "histogram",
            Algorithm::Myers => "myers",
            Algorithm::MyersMinimal => "myers_minimal",
        }
    }
    #[getter]
    fn conflict(&self) -> &'static str {
        use blob::builtin_driver::text::Conflict;
        match self.inner.text.conflict {
            Conflict::Keep { .. } => "keep",
            Conflict::ResolveWithOurs => "ours",
            Conflict::ResolveWithTheirs => "theirs",
            Conflict::ResolveWithUnion => "union",
        }
    }
    #[getter]
    fn conflict_style(&self) -> Option<&'static str> {
        use blob::builtin_driver::text::{Conflict, ConflictStyle};
        match self.inner.text.conflict {
            Conflict::Keep { style, .. } => Some(match style {
                ConflictStyle::Merge => "merge",
                ConflictStyle::Diff3 => "diff3",
                ConflictStyle::ZealousDiff3 => "zdiff3",
            }),
            _ => None,
        }
    }
    #[getter]
    fn marker_size(&self) -> Option<u8> {
        self.inner.text.conflict.marker_size()
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct MergeDriver {
    inner: blob::Driver,
}
#[pymethods]
impl MergeDriver {
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.name)
    }
    #[getter]
    fn display_name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.display_name)
    }
    #[getter]
    fn command<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.command)
    }
    #[getter]
    fn recursive<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner.recursive.as_ref().map(|v| PyBytes::new(py, v))
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct MergeResourceCache {
    handle: RepoHandle,
    inner: Arc<Mutex<blob::Platform>>,
}
impl MergeResourceCache {
    fn with<T: Send + 'static>(
        &self,
        py: Python<'_>,
        work: impl FnOnce(&mut blob::Platform, &gix::Repository) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let inner = self.inner.clone();
        self.handle.run(py, move |repo| {
            let mut cache = inner
                .try_lock()
                .map_err(|_| PyRuntimeError::new_err("merge resource cache is already in use"))?;
            work(&mut cache, repo)
        })
    }
}
#[pymethods]
impl MergeResourceCache {
    fn set_resource(
        &self,
        py: Python<'_>,
        id: &Bound<'_, PyAny>,
        mode: u32,
        rela_path: &Bound<'_, PyAny>,
        kind: &str,
    ) -> PyResult<()> {
        let id = ObjectSpec::extract(id)?;
        let path = bytes(rela_path)?;
        let kind = resource_kind(kind)?;
        let mode = gix::objs::tree::EntryMode::try_from(mode)
            .map_err(|_| PyValueError::new_err("invalid entry mode"))?
            .kind();
        self.with(py, move |cache, repo| {
            cache
                .set_resource(id.resolve(repo)?, mode, path.as_bstr(), kind, &repo.objects)
                .map_err(to_py)
        })
    }
    fn drivers(&self, py: Python<'_>) -> PyResult<Vec<MergeDriver>> {
        self.with(py, |cache, _| {
            Ok(cache
                .drivers()
                .iter()
                .cloned()
                .map(|inner| MergeDriver { inner })
                .collect())
        })
    }
    #[getter]
    fn default_driver<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        Ok(self
            .with(py, |cache, _| Ok(cache.options.default_driver.clone()))?
            .map(|v| PyBytes::new(py, &v)))
    }
    #[setter]
    fn set_default_driver(&self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let value = value.map(bytes).transpose()?;
        self.with(py, move |cache, _| {
            cache.options.default_driver = value.map(Into::into);
            Ok(())
        })
    }
    #[getter]
    fn filter_mode(&self, py: Python<'_>) -> PyResult<&'static str> {
        self.with(py, |cache, _| {
            Ok(match cache.filter_mode {
                blob::pipeline::Mode::ToGit => "to_git",
                blob::pipeline::Mode::Renormalize => "renormalize",
            })
        })
    }
    #[setter]
    fn set_filter_mode(&self, py: Python<'_>, value: &str) -> PyResult<()> {
        let mode = match value {
            "to_git" => blob::pipeline::Mode::ToGit,
            "renormalize" => blob::pipeline::Mode::Renormalize,
            _ => return Err(PyValueError::new_err("filter mode must be to_git or renormalize")),
        };
        self.with(py, move |cache, _| {
            cache.filter_mode = mode;
            Ok(())
        })
    }
    #[getter]
    fn large_file_threshold_bytes(&self, py: Python<'_>) -> PyResult<u64> {
        self.with(py, |cache, _| Ok(cache.filter.options.large_file_threshold_bytes))
    }
    #[setter]
    fn set_large_file_threshold_bytes(&self, py: Python<'_>, value: u64) -> PyResult<()> {
        self.with(py, move |cache, _| {
            cache.filter.options.large_file_threshold_bytes = value;
            Ok(())
        })
    }
    #[pyo3(signature=(options=None,*,progress=None,cancel=None))]
    fn prepare_merge(
        &self,
        py: Python<'_>,
        options: Option<BlobMergeOptions>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<PreparedMerge> {
        let handle = self.handle.clone();
        let inner = self.inner.clone();
        let mut owner =
            Owner::new_with_options("prepare blob merge", progress, cancel, move |_, commands, producer| {
                handle.with(|repo| {
                    let mut cache = inner
                        .try_lock()
                        .map_err(|_| PyRuntimeError::new_err("merge resource cache is already in use"))?;
                    let options = match options {
                        Some(v) => v.inner,
                        None => repo.blob_merge_options().map_err(to_py)?,
                    };
                    let mut prepared = cache.prepare_merge(&repo.objects, options).map_err(to_py)?;
                    producer.serve(std::iter::from_fn(move || {
                        let job = commands.lock().unwrap_or_else(|e| e.into_inner()).take()?;
                        Some(Ok(job(&mut prepared, repo)))
                    }))
                })
            });
        downcast::<()>(owner.call(py, Box::new(|_, _| Ok(Box::new(()) as Value)))?)?;
        owner.finish_initialization();
        Ok(PreparedMerge { owner: Arc::new(owner) })
    }
}

type Value = Box<dyn Any + Send>;
type Job = Box<dyn for<'a> FnOnce(&mut blob::PlatformRef<'a>, &gix::Repository) -> PyResult<Value> + Send>;
type Owner = CommandOwner<Job, Value>;
fn downcast<T: Send + 'static>(value: Value) -> PyResult<T> {
    value
        .downcast::<T>()
        .map(|v| *v)
        .map_err(|_| PyRuntimeError::new_err("unexpected native merge response"))
}
fn call<T: Send + 'static>(
    owner: &Owner,
    py: Python<'_>,
    work: impl FnOnce(&mut blob::PlatformRef<'_>, &gix::Repository) -> PyResult<T> + Send + 'static,
) -> PyResult<T> {
    downcast(owner.call(
        py,
        Box::new(move |prepared, repo| work(prepared, repo).map(|v| Box::new(v) as Value)),
    )?)
}
fn resource<'a>(prepared: &blob::PlatformRef<'a>, kind: blob::ResourceKind) -> blob::platform::ResourceRef<'a> {
    match kind {
        blob::ResourceKind::CurrentOrOurs => prepared.current,
        blob::ResourceKind::CommonAncestorOrBase => prepared.ancestor,
        blob::ResourceKind::OtherOrTheirs => prepared.other,
    }
}
#[pyclass(frozen, module = "gix")]
pub struct MergeResource {
    owner: Arc<Owner>,
    role: blob::ResourceKind,
    #[pyo3(get)]
    id: ObjectId,
    path: Vec<u8>,
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    size: Option<u64>,
}
#[pymethods]
impl MergeResource {
    #[getter]
    fn rela_path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.path)
    }
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let role = self.role;
        let data = call(&self.owner, py, move |prepared, _| {
            Ok(match resource(prepared, role).data {
                blob::platform::resource::Data::Buffer(data) => Some(data.to_vec()),
                _ => None,
            })
        })?;
        Ok(data.map(|v| PyBytes::new(py, &v)))
    }
    fn as_slice<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let role = self.role;
        let data = call(&self.owner, py, move |prepared, _| {
            Ok(resource(prepared, role).data.as_slice().map(|v| v.to_vec()))
        })?;
        Ok(data.map(|v| PyBytes::new(py, &v)))
    }
}
#[pyclass(frozen, module = "gix")]
pub struct PreparedMerge {
    owner: Arc<Owner>,
}
impl PreparedMerge {
    fn resource(&self, py: Python<'_>, role: blob::ResourceKind) -> PyResult<MergeResource> {
        let (id, path, kind, size) = call(&self.owner, py, move |prepared, _| {
            let r = resource(prepared, role);
            use blob::platform::resource::Data;
            let (kind, size) = match r.data {
                Data::Missing => ("Missing", None),
                Data::Buffer(v) => ("Buffer", Some(v.len() as u64)),
                Data::TooLarge { size } => ("TooLarge", Some(size)),
            };
            Ok((ObjectId { inner: r.id.to_owned() }, r.rela_path.to_vec(), kind, size))
        })?;
        Ok(MergeResource {
            owner: self.owner.clone(),
            role,
            id,
            path,
            kind,
            size,
        })
    }
}
#[pymethods]
impl PreparedMerge {
    #[getter]
    fn current(&self, py: Python<'_>) -> PyResult<MergeResource> {
        self.resource(py, blob::ResourceKind::CurrentOrOurs)
    }
    #[getter]
    fn ancestor(&self, py: Python<'_>) -> PyResult<MergeResource> {
        self.resource(py, blob::ResourceKind::CommonAncestorOrBase)
    }
    #[getter]
    fn other(&self, py: Python<'_>) -> PyResult<MergeResource> {
        self.resource(py, blob::ResourceKind::OtherOrTheirs)
    }
    #[getter]
    fn options(&self, py: Python<'_>) -> PyResult<BlobMergeOptions> {
        call(&self.owner, py, |p, _| Ok(BlobMergeOptions { inner: p.options }))
    }
    #[setter]
    fn set_options(&self, py: Python<'_>, value: BlobMergeOptions) -> PyResult<()> {
        call(&self.owner, py, move |p, _| {
            p.options = value.inner;
            Ok(())
        })
    }
    #[getter]
    fn driver(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value = call(&self.owner, py, |p, _| Ok(p.driver))?;
        match value {
            blob::platform::DriverChoice::BuiltIn(v) => Ok(v.as_str().into_pyobject(py)?.into_any().unbind()),
            blob::platform::DriverChoice::Index(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        }
    }
    #[setter]
    fn set_driver(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = if let Ok(value) = value.extract::<String>() {
            blob::platform::DriverChoice::BuiltIn(driver(&value)?)
        } else {
            blob::platform::DriverChoice::Index(value.extract::<usize>()?)
        };
        call(&self.owner, py, move |p, _| {
            p.driver = value;
            Ok(())
        })
    }
    fn configured_driver(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value = call(&self.owner, py, |p, _| {
            Ok(p.configured_driver().map(|inner| MergeDriver { inner: inner.clone() }))
        })?;
        match value {
            Ok(value) => Ok(Py::new(py, value)?.into_any()),
            Err(value) => Ok(value.as_str().into_pyobject(py)?.into_any().unbind()),
        }
    }
    #[pyo3(signature=(labels=None))]
    fn merge<'py>(
        &self,
        py: Python<'py>,
        labels: Option<MergeLabels>,
    ) -> PyResult<(Bound<'py, PyBytes>, String, String)> {
        let labels = labels.unwrap_or_default();
        let (out, pick, resolution) = call(&self.owner, py, move |p, repo| {
            let mut out = Vec::new();
            let (pick, resolution) = p
                .merge(&mut out, labels.native(), &repo.command_context().map_err(to_py)?)
                .map_err(to_py)?;
            Ok((out, format!("{pick:?}"), format!("{resolution:?}")))
        })?;
        Ok((PyBytes::new(py, &out), pick, resolution))
    }
    #[pyo3(signature=(driver,labels=None))]
    fn builtin_merge<'py>(
        &self,
        py: Python<'py>,
        driver: &str,
        labels: Option<MergeLabels>,
    ) -> PyResult<(Bound<'py, PyBytes>, String, String)> {
        let driver = crate::blob_merge::driver(driver)?;
        let labels = labels.unwrap_or_default();
        let (out, pick, resolution) = call(&self.owner, py, move |p, _| {
            let mut out = Vec::new();
            let (pick, resolution) = p.builtin_merge(driver, &mut out, &mut Default::default(), labels.native());
            Ok((out, format!("{pick:?}"), format!("{resolution:?}")))
        })?;
        Ok((PyBytes::new(py, &out), pick, resolution))
    }
    fn buffer_by_pick<'py>(&self, py: Python<'py>, value: &str) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let pick = pick(value)?;
        let out = call(&self.owner, py, move |p, _| {
            p.buffer_by_pick(pick)
                .map(|v| v.map(|b| b.to_vec()))
                .map_err(|()| PyValueError::new_err("selected merge resource exceeds the large-file threshold"))
        })?;
        Ok(out.map(|v| PyBytes::new(py, &v)))
    }
    fn id_by_pick(&self, py: Python<'_>, value: &str, buffer: &Bound<'_, PyBytes>) -> PyResult<Option<ObjectId>> {
        let pick = pick(value)?;
        let buffer = buffer.as_bytes().to_vec();
        call(&self.owner, py, move |p, repo| {
            p.id_by_pick(pick, &buffer, |data| {
                repo.write_blob(data).map(|v| v.detach()).map_err(to_py)
            })
            .map(|v| v.map(|inner| ObjectId { inner }))
        })
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
    fn blob_merge_options(&self, py: Python<'_>) -> PyResult<BlobMergeOptions> {
        self.handle.run(py, |repo| {
            repo.blob_merge_options()
                .map(|inner| BlobMergeOptions { inner })
                .map_err(to_py)
        })
    }
    #[pyo3(signature=(*,current_root=None,other_root=None,common_ancestor_root=None,progress=None,cancel=None))]
    fn merge_resource_cache(
        &self,
        py: Python<'_>,
        current_root: Option<PathBuf>,
        other_root: Option<PathBuf>,
        common_ancestor_root: Option<PathBuf>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<MergeResourceCache> {
        let handle = self.handle.clone();
        let work_handle = handle.clone();
        let inner = runtime::run(py, "merge resource cache", progress, cancel, move |_| {
            work_handle.with(|repo| {
                repo.merge_resource_cache(blob::pipeline::WorktreeRoots {
                    current_root,
                    other_root,
                    common_ancestor_root,
                })
                .map_err(to_py)
            })
        })??;
        Ok(MergeResourceCache {
            handle,
            inner: Arc::new(Mutex::new(inner)),
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<BlobMergeOptions>()?;
    m.add_class::<MergeDriver>()?;
    m.add_class::<MergeResourceCache>()?;
    m.add_class::<MergeResource>()?;
    m.add_class::<PreparedMerge>()?;
    Ok(())
}
