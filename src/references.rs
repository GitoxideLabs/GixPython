//! Owned reference snapshots, native reference transactions, and lazy cursors.

#![allow(clippy::wrong_self_convention, reason = "Preserve native API names without consuming Python objects")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gix::{bstr::ByteSlice, prelude::ReferenceExt};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};

use crate::{
    error::to_py,
    objects::{Blob, Commit, Object, Signature, Tag, Tree},
    repository::{RepoHandle, Repository},
    runtime::{CancellationToken, OwnedIter, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};

fn full_name(value: &Bound<'_, PyAny>) -> PyResult<gix::refs::FullName> {
    gix::refs::FullName::try_from(bytes(value)?.as_bstr()).map_err(to_py)
}

pub(crate) fn validate_target(repo: &gix::Repository, target: &gix::refs::Target) -> PyResult<()> {
    if target.try_id().is_some_and(|id| id.kind() != repo.object_hash()) {
        return Err(PyValueError::new_err(
            "reference target hash kind differs from the repository",
        ));
    }
    Ok(())
}

fn validate_edit(repo: &gix::Repository, edit: &gix::refs::transaction::RefEdit) -> PyResult<()> {
    if matches!(
        edit.change,
        gix::refs::transaction::Change::Delete {
            expected: gix::refs::transaction::PreviousValue::MustNotExist,
            ..
        }
    ) {
        return Err(PyValueError::new_err("MustNotExist is invalid for deletion"));
    }
    if let Some(target) = edit.change.new_value() {
        validate_target(repo, &target.into_owned())?;
    }
    if let Some(target) = edit.change.previous_value() {
        validate_target(repo, &target.into_owned())?;
    }
    Ok(())
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct Target {
    pub inner: gix::refs::Target,
}

#[pymethods]
impl Target {
    #[staticmethod]
    #[pyo3(name = "Object")]
    fn object(id: ObjectId) -> Self {
        Self {
            inner: gix::refs::Target::Object(id.inner),
        }
    }
    #[staticmethod]
    #[pyo3(name = "Symbolic")]
    fn symbolic(name: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: gix::refs::Target::Symbolic(full_name(name)?),
        })
    }
    fn try_id(&self) -> Option<ObjectId> {
        self.inner.try_id().map(|id| ObjectId { inner: id.to_owned() })
    }
    fn try_name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        match &self.inner {
            gix::refs::Target::Symbolic(name) => Some(PyBytes::new(py, name.as_bstr())),
            _ => None,
        }
    }
    fn __repr__(&self) -> String {
        format!("Target({:?})", self.inner)
    }
}

/// Constraints contain literal snapshots, never revspecs evaluated during a write.
#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PreviousValue {
    pub inner: gix::refs::transaction::PreviousValue,
}

#[pymethods]
impl PreviousValue {
    #[classattr]
    #[pyo3(name = "Any")]
    fn any() -> Self {
        Self {
            inner: gix::refs::transaction::PreviousValue::Any,
        }
    }
    #[classattr]
    #[pyo3(name = "MustExist")]
    fn must_exist() -> Self {
        Self {
            inner: gix::refs::transaction::PreviousValue::MustExist,
        }
    }
    #[classattr]
    #[pyo3(name = "MustNotExist")]
    fn must_not_exist() -> Self {
        Self {
            inner: gix::refs::transaction::PreviousValue::MustNotExist,
        }
    }
    #[staticmethod]
    #[pyo3(name = "MustExistAndMatch")]
    fn must_exist_and_match(target: Target) -> Self {
        Self {
            inner: gix::refs::transaction::PreviousValue::MustExistAndMatch(target.inner),
        }
    }
    #[staticmethod]
    #[pyo3(name = "ExistingMustMatch")]
    fn existing_must_match(target: Target) -> Self {
        Self {
            inner: gix::refs::transaction::PreviousValue::ExistingMustMatch(target.inner),
        }
    }
    fn __repr__(&self) -> String {
        format!("PreviousValue::{:?}", self.inner)
    }
}

#[pyclass(frozen, module = "gix", from_py_object, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RefLog {
    pub inner: gix::refs::transaction::RefLog,
}

#[pymethods]
impl RefLog {
    #[classattr]
    #[pyo3(name = "AndReference")]
    fn and_reference() -> Self {
        Self {
            inner: gix::refs::transaction::RefLog::AndReference,
        }
    }
    #[classattr]
    #[pyo3(name = "Only")]
    fn only() -> Self {
        Self {
            inner: gix::refs::transaction::RefLog::Only,
        }
    }
    fn __repr__(&self) -> String {
        format!("RefLog::{:?}", self.inner)
    }
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct LogChange {
    pub inner: gix::refs::transaction::LogChange,
}

#[pymethods]
impl LogChange {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    #[getter]
    fn mode(&self) -> RefLog {
        RefLog { inner: self.inner.mode }
    }
    #[setter]
    fn set_mode(&mut self, mode: RefLog) {
        self.inner.mode = mode.inner;
    }
    #[getter]
    fn force_create_reflog(&self) -> bool {
        self.inner.force_create_reflog
    }
    #[setter]
    fn set_force_create_reflog(&mut self, value: bool) {
        self.inner.force_create_reflog = value;
    }
    #[getter]
    fn message<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.message)
    }
    #[setter]
    fn set_message(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.message = bytes(value)?.into();
        Ok(())
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Change {
    pub inner: gix::refs::transaction::Change,
}

#[pymethods]
impl Change {
    #[staticmethod]
    #[pyo3(name = "Update")]
    fn update(log: LogChange, expected: PreviousValue, new: Target) -> Self {
        Self {
            inner: gix::refs::transaction::Change::Update {
                log: log.inner,
                expected: expected.inner,
                new: new.inner,
            },
        }
    }
    #[staticmethod]
    #[pyo3(name = "Delete")]
    fn delete(expected: PreviousValue, log: RefLog) -> Self {
        Self {
            inner: gix::refs::transaction::Change::Delete {
                expected: expected.inner,
                log: log.inner,
            },
        }
    }
    fn new_value(&self) -> Option<Target> {
        self.inner.new_value().map(|v| Target { inner: v.into_owned() })
    }
    fn previous_value(&self) -> Option<Target> {
        self.inner.previous_value().map(|v| Target { inner: v.into_owned() })
    }
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RefEdit {
    pub inner: gix::refs::transaction::RefEdit,
}

#[pymethods]
impl RefEdit {
    #[new]
    fn new(name: &Bound<'_, PyAny>, change: Change) -> PyResult<Self> {
        Ok(Self {
            inner: gix::refs::transaction::RefEdit::new(full_name(name)?, change.inner),
        })
    }
    #[staticmethod]
    fn update(
        name: &Bound<'_, PyAny>,
        new: Target,
        expected: PreviousValue,
        reflog_message: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: gix::refs::transaction::RefEdit::update(
                full_name(name)?,
                new.inner,
                expected.inner,
                bytes(reflog_message)?,
            ),
        })
    }
    #[staticmethod]
    fn update_with_log(
        name: &Bound<'_, PyAny>,
        new: Target,
        expected: PreviousValue,
        log: LogChange,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: gix::refs::transaction::RefEdit::update_with_log(
                full_name(name)?,
                new.inner,
                expected.inner,
                log.inner,
            ),
        })
    }
    #[staticmethod]
    fn delete(name: &Bound<'_, PyAny>, expected: PreviousValue) -> PyResult<Self> {
        if matches!(expected.inner, gix::refs::transaction::PreviousValue::MustNotExist) {
            return Err(PyValueError::new_err("MustNotExist is invalid for deletion"));
        }
        Ok(Self {
            inner: gix::refs::transaction::RefEdit::delete(full_name(name)?, expected.inner),
        })
    }
    #[staticmethod]
    fn delete_with_log(name: &Bound<'_, PyAny>, expected: PreviousValue, log: RefLog) -> PyResult<Self> {
        let mut edit = Self::delete(name, expected)?;
        edit.inner = gix::refs::transaction::RefEdit::delete_with_log(
            edit.inner.name,
            match edit.inner.change {
                gix::refs::transaction::Change::Delete { expected, .. } => expected,
                _ => unreachable!("delete produces a deletion"),
            },
            log.inner,
        );
        Ok(edit)
    }
    fn with_deref(&self, deref: bool) -> Self {
        Self {
            inner: self.inner.clone().with_deref(deref),
        }
    }
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.name.as_bstr())
    }
    #[getter]
    fn change(&self) -> Change {
        Change {
            inner: self.inner.change.clone(),
        }
    }
    #[getter]
    fn deref(&self) -> bool {
        self.inner.deref
    }
    #[setter]
    fn set_deref(&mut self, deref: bool) {
        self.inner.deref = deref;
    }
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Reference {
    pub handle: RepoHandle,
    pub inner: gix::refs::Reference,
}

impl Reference {
    pub fn from_native(handle: RepoHandle, reference: gix::Reference<'_>) -> Self {
        Self::from_detached(handle, reference.detach())
    }
    pub fn from_detached(handle: RepoHandle, inner: gix::refs::Reference) -> Self {
        Self { handle, inner }
    }
    fn native_mut<T: Send + 'static>(
        &mut self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::Reference<'_>) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let inner = self.inner.clone();
        let (result, inner) = self.handle.run(py, move |repo| {
            let mut reference = inner.attach(repo);
            let result = work(&mut reference);
            Ok((result, reference.detach()))
        })?;
        self.inner = inner;
        result
    }
    fn peel(&mut self, py: Python<'_>, kind: gix::objs::Kind) -> PyResult<Object> {
        let handle = self.handle.clone();
        self.native_mut(py, move |reference| {
            reference
                .peel_to_kind(kind)
                .map(|object| Object::from_native(handle, object))
                .map_err(to_py)
        })
    }
}

#[pymethods]
impl Reference {
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.name.as_bstr())
    }
    fn target(&self) -> Target {
        Target {
            inner: self.inner.target.clone(),
        }
    }
    fn try_id(&self) -> Option<ObjectId> {
        self.inner.target.try_id().map(|id| ObjectId { inner: id.to_owned() })
    }
    fn id(&self) -> PyResult<ObjectId> {
        self.try_id()
            .ok_or_else(|| PyValueError::new_err("symbolic references have no direct object ID"))
    }
    fn follow(&self, py: Python<'_>) -> PyResult<Option<Self>> {
        let inner = self.inner.clone();
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            inner
                .attach(repo)
                .follow()
                .transpose()
                .map(|value| value.map(|r| Self::from_native(handle, r)))
                .map_err(to_py)
        })
    }
    fn follow_to_object(&mut self, py: Python<'_>) -> PyResult<ObjectId> {
        self.native_mut(py, |r| {
            r.follow_to_object()
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }
    fn peel_to_id(&mut self, py: Python<'_>) -> PyResult<ObjectId> {
        self.native_mut(py, |r| {
            r.peel_to_id().map(|id| ObjectId { inner: id.detach() }).map_err(to_py)
        })
    }
    fn into_fully_peeled_id(&self, py: Python<'_>) -> PyResult<ObjectId> {
        self.clone().peel_to_id(py)
    }
    fn peel_to_kind(&mut self, py: Python<'_>, kind: &str) -> PyResult<Object> {
        let kind = match kind {
            "blob" => gix::objs::Kind::Blob,
            "tree" => gix::objs::Kind::Tree,
            "commit" => gix::objs::Kind::Commit,
            "tag" => gix::objs::Kind::Tag,
            _ => return Err(PyValueError::new_err("expected blob, tree, commit, or tag")),
        };
        self.peel(py, kind)
    }
    fn peel_to_commit(&mut self, py: Python<'_>) -> PyResult<Commit> {
        self.peel(py, gix::objs::Kind::Commit).map(|object| Commit { object })
    }
    fn peel_to_tree(&mut self, py: Python<'_>) -> PyResult<Tree> {
        self.peel(py, gix::objs::Kind::Tree).map(|object| Tree { object })
    }
    fn peel_to_blob(&mut self, py: Python<'_>) -> PyResult<Blob> {
        self.peel(py, gix::objs::Kind::Blob).map(|object| Blob { object })
    }
    fn peel_to_tag(&mut self, py: Python<'_>) -> PyResult<Tag> {
        self.peel(py, gix::objs::Kind::Tag).map(|object| Tag { object })
    }
    fn set_target_id(
        &mut self,
        py: Python<'_>,
        id: &Bound<'_, PyAny>,
        reflog_message: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let id = ObjectSpec::extract(id)?;
        let message = bytes(reflog_message)?;
        self.native_mut(py, move |r| {
            r.set_target_id(id.resolve(r.repo)?, message).map_err(to_py)
        })
    }
    fn delete(&self, py: Python<'_>) -> PyResult<()> {
        let inner = self.inner.clone();
        self.handle
            .run(py, move |repo| inner.attach(repo).delete().map_err(to_py))
    }
    fn log_exists(&self, py: Python<'_>) -> PyResult<bool> {
        let inner = self.inner.clone();
        self.handle.run(py, move |repo| Ok(inner.attach(repo).log_exists()))
    }
    fn log_iter(&self) -> ReflogIterPlatform {
        ReflogIterPlatform {
            handle: self.handle.clone(),
            name: self.inner.name.clone(),
        }
    }
    fn __repr__(&self) -> String {
        format!("Reference({:?})", self.inner)
    }
}

#[pyclass(module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Head {
    pub(crate) handle: RepoHandle,
    pub(crate) inner: gix::head::Kind,
}

impl Head {
    fn native_mut<T: Send + 'static>(
        &mut self,
        py: Python<'_>,
        work: impl FnOnce(&mut gix::Head<'_>) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let inner = self.inner.clone();
        let (result, inner) = self.handle.run(py, move |repo| {
            let mut head = inner.attach(repo);
            let result = work(&mut head);
            Ok((result, head.kind))
        })?;
        self.inner = inner;
        result
    }
}

#[pymethods]
impl Head {
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, b"HEAD")
    }
    fn referent_name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        let name = match &self.inner {
            gix::head::Kind::Symbolic(reference) => &reference.name,
            gix::head::Kind::Unborn(name) => name,
            gix::head::Kind::Detached { .. } => return None,
        };
        Some(PyBytes::new(py, name.as_bstr()))
    }
    fn is_detached(&self) -> bool {
        matches!(self.inner, gix::head::Kind::Detached { .. })
    }
    fn is_unborn(&self) -> bool {
        matches!(self.inner, gix::head::Kind::Unborn(_))
    }
    fn id(&self) -> Option<ObjectId> {
        let inner = match &self.inner {
            gix::head::Kind::Symbolic(r) => r.target.try_id()?.to_owned(),
            gix::head::Kind::Detached { target, peeled } => peeled.unwrap_or(*target),
            gix::head::Kind::Unborn(_) => return None,
        };
        Some(ObjectId { inner })
    }
    fn try_into_referent(&self) -> Option<Reference> {
        match &self.inner {
            gix::head::Kind::Symbolic(inner) => Some(Reference::from_detached(self.handle.clone(), inner.clone())),
            _ => None,
        }
    }
    fn try_peel_to_id(&mut self, py: Python<'_>) -> PyResult<Option<ObjectId>> {
        self.native_mut(py, |h| {
            h.try_peel_to_id()
                .map(|v| v.map(|id| ObjectId { inner: id.detach() }))
                .map_err(to_py)
        })
    }
    fn try_into_peeled_id(&self, py: Python<'_>) -> PyResult<Option<ObjectId>> {
        self.clone().try_peel_to_id(py)
    }
    fn into_peeled_id(&self, py: Python<'_>) -> PyResult<ObjectId> {
        let inner = self.inner.clone();
        self.handle.run(py, move |r| {
            inner
                .attach(r)
                .into_peeled_id()
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }
    fn peel_to_object(&mut self, py: Python<'_>) -> PyResult<Object> {
        let handle = self.handle.clone();
        self.native_mut(py, move |h| {
            h.peel_to_object()
                .map(|o| Object::from_native(handle, o))
                .map_err(to_py)
        })
    }
    fn into_peeled_object(&self, py: Python<'_>) -> PyResult<Object> {
        self.clone().peel_to_object(py)
    }
    fn peel_to_commit(&mut self, py: Python<'_>) -> PyResult<Commit> {
        let handle = self.handle.clone();
        self.native_mut(py, move |h| {
            h.peel_to_commit()
                .map(|o| Commit {
                    object: Object::from_detached(handle, o.detach()),
                })
                .map_err(to_py)
        })
    }
    fn log_iter(&self) -> PyResult<ReflogIterPlatform> {
        Ok(ReflogIterPlatform {
            handle: self.handle.clone(),
            name: gix::refs::FullName::try_from("HEAD").map_err(to_py)?,
        })
    }
    #[allow(clippy::type_complexity, reason = "Preserve the native optional list of branch names and IDs")]
    fn prior_checked_out_branches(&self, py: Python<'_>) -> PyResult<Option<Vec<(Py<PyBytes>, ObjectId)>>> {
        let inner = self.inner.clone();
        let values = self
            .handle
            .run(py, move |r| inner.attach(r).prior_checked_out_branches().map_err(to_py))?;
        Ok(values.map(|values| {
            values
                .into_iter()
                .map(|(name, inner)| (PyBytes::new(py, &name).unbind(), ObjectId { inner }))
                .collect()
        }))
    }
}

#[derive(Clone)]
enum RefSelection {
    All,
    LocalBranches,
    RemoteBranches,
    Tags,
    Pseudo,
    Prefix(Vec<u8>),
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct ReferenceIterPlatform {
    handle: RepoHandle,
}

impl ReferenceIterPlatform {
    fn iter(
        &self,
        selection: RefSelection,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> ReferenceIter {
        let handle = self.handle.clone();
        let peel = Arc::new(AtomicBool::new(false));
        let should_peel = peel.clone();
        ReferenceIter {
            handle: self.handle.clone(),
            peel,
            inner: OwnedIter::new("references", progress, cancel, move |_, producer| {
                handle.with(|repo| {
                    let packed = repo.refs.cached_packed_buffer().map_err(to_py)?;
                    let platform = repo.references().map_err(to_py)?;
                    let iter = match &selection {
                        RefSelection::All => platform.all(),
                        RefSelection::LocalBranches => platform.local_branches(),
                        RefSelection::RemoteBranches => platform.remote_branches(),
                        RefSelection::Tags => platform.tags(),
                        RefSelection::Pseudo => platform.pseudo(),
                        RefSelection::Prefix(prefix) => platform.prefixed(prefix.as_bstr()),
                    }
                    .map_err(to_py)?;
                    producer.serve(iter.map(|item| {
                        let mut reference = item.map_err(to_py)?;
                        if should_peel.load(Ordering::Acquire) {
                            reference
                                .peel_to_id_packed(packed.as_ref().map(|p| &***p))
                                .map_err(to_py)?;
                        }
                        Ok(reference.detach())
                    }))
                })
            }),
        }
    }
}

#[pymethods]
impl ReferenceIterPlatform {
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn all(&self, progress: Option<Progress>, cancel: Option<CancellationToken>) -> ReferenceIter {
        self.iter(RefSelection::All, progress.as_ref(), cancel.as_ref())
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn local_branches(&self, progress: Option<Progress>, cancel: Option<CancellationToken>) -> ReferenceIter {
        self.iter(RefSelection::LocalBranches, progress.as_ref(), cancel.as_ref())
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn remote_branches(&self, progress: Option<Progress>, cancel: Option<CancellationToken>) -> ReferenceIter {
        self.iter(RefSelection::RemoteBranches, progress.as_ref(), cancel.as_ref())
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn tags(&self, progress: Option<Progress>, cancel: Option<CancellationToken>) -> ReferenceIter {
        self.iter(RefSelection::Tags, progress.as_ref(), cancel.as_ref())
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn pseudo(&self, progress: Option<Progress>, cancel: Option<CancellationToken>) -> ReferenceIter {
        self.iter(RefSelection::Pseudo, progress.as_ref(), cancel.as_ref())
    }
    #[pyo3(signature = (prefix, *, progress=None, cancel=None))]
    fn prefixed(
        &self,
        prefix: &Bound<'_, PyAny>,
        progress: Option<Progress>,
        cancel: Option<CancellationToken>,
    ) -> PyResult<ReferenceIter> {
        Ok(self.iter(RefSelection::Prefix(bytes(prefix)?), progress.as_ref(), cancel.as_ref()))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct ReferenceIter {
    handle: RepoHandle,
    inner: OwnedIter<gix::refs::Reference, PyErr>,
    peel: Arc<AtomicBool>,
}

#[pymethods]
impl ReferenceIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Reference>> {
        self.inner
            .next(py)?
            .transpose()
            .map(|value| value.map(|inner| Reference::from_detached(self.handle.clone(), inner)))
    }
    fn peeled(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf.peel.store(true, Ordering::Release);
        slf
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

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct ReflogIterPlatform {
    handle: RepoHandle,
    name: gix::refs::FullName,
}

#[pymethods]
impl ReflogIterPlatform {
    /// Native forward reflog iteration reads the file, then parses entries lazily.
    fn all(&self, py: Python<'_>) -> PyResult<Option<ReflogIter>> {
        let name = self.name.clone();
        let data = self.handle.run(py, move |repo| {
            let mut data = Vec::new();
            let exists = repo
                .refs
                .reflog_iter(name.as_ref(), &mut data)
                .map_err(to_py)?
                .is_some();
            Ok(exists.then_some(data))
        })?;
        Ok(data.map(|data| ReflogIter {
            inner: OwnedIter::new("reflog", None, None, move |_, producer| {
                producer.serve(
                    gix::refs::file::log::iter::forward(&data)
                        .map(|line| line.map(|line| line.to_owned()).map_err(to_py)),
                )
            }),
        }))
    }
    /// Native reverse iteration uses a bounded sliding buffer, newest entry first.
    fn rev(&self, py: Python<'_>) -> PyResult<Option<ReflogIter>> {
        let name = self.name.clone();
        let exists = self.handle.run(py, move |repo| {
            repo.refs
                .reflog_iter_rev(name.as_ref(), &mut [0; 4096])
                .map(|iter| iter.is_some())
                .map_err(to_py)
        })?;
        if !exists {
            return Ok(None);
        }
        let handle = self.handle.clone();
        let name = self.name.clone();
        Ok(Some(ReflogIter {
            inner: OwnedIter::new("reflog", None, None, move |_, producer| {
                handle.with(|repo| {
                    let mut buffer = [0; 4096];
                    let iter = repo.refs.reflog_iter_rev(name.as_ref(), &mut buffer).map_err(to_py)?;
                    producer.serve(iter.into_iter().flatten().map(|line| line.map_err(to_py)))
                })
            }),
        }))
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct ReflogLine {
    inner: gix::refs::log::Line,
}

#[pymethods]
impl ReflogLine {
    #[getter]
    fn previous_oid(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.previous_oid,
        }
    }
    #[getter]
    fn new_oid(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.new_oid,
        }
    }
    #[getter]
    fn signature(&self) -> Signature {
        Signature {
            inner: self.inner.signature.clone(),
        }
    }
    #[getter]
    fn message<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.message)
    }
}

#[pyclass(frozen, module = "gix")]
pub struct ReflogIter {
    inner: OwnedIter<gix::refs::log::Line, PyErr>,
}

#[pymethods]
impl ReflogIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<ReflogLine>> {
        self.inner
            .next(py)?
            .transpose()
            .map(|value| value.map(|inner| ReflogLine { inner }))
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
    fn head(&self, py: Python<'_>) -> PyResult<Head> {
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.head()
                .map(|head| Head {
                    handle,
                    inner: head.kind,
                })
                .map_err(to_py)
        })
    }
    fn head_id(&self, py: Python<'_>) -> PyResult<ObjectId> {
        self.handle.run(py, |r| {
            r.head_id().map(|id| ObjectId { inner: id.detach() }).map_err(to_py)
        })
    }
    fn head_name<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let name = self.handle.run(py, |r| {
            r.head_name()
                .map(|name| name.map(|n| n.as_bstr().to_vec()))
                .map_err(to_py)
        })?;
        Ok(name.map(|name| PyBytes::new(py, &name)))
    }
    fn head_ref(&self, py: Python<'_>) -> PyResult<Option<Reference>> {
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.head_ref()
                .map(|value| value.map(|reference| Reference::from_native(handle, reference)))
                .map_err(to_py)
        })
    }
    fn head_commit(&self, py: Python<'_>) -> PyResult<Commit> {
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.head_commit()
                .map(|o| Commit {
                    object: Object::from_detached(handle, o.detach()),
                })
                .map_err(to_py)
        })
    }
    fn head_tree_id(&self, py: Python<'_>) -> PyResult<ObjectId> {
        self.handle.run(py, |r| {
            r.head_tree_id()
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }
    fn head_tree_id_or_empty(&self, py: Python<'_>) -> PyResult<ObjectId> {
        self.handle.run(py, |r| {
            r.head_tree_id_or_empty()
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }
    fn head_tree(&self, py: Python<'_>) -> PyResult<Tree> {
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.head_tree()
                .map(|o| Tree {
                    object: Object::from_detached(handle, o.detach()),
                })
                .map_err(to_py)
        })
    }
    fn find_reference(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Reference> {
        let name = bytes(name)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.find_reference(name.as_bstr())
                .map(|reference| Reference::from_native(handle, reference))
                .map_err(to_py)
        })
    }
    fn try_find_reference(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<Reference>> {
        let name = bytes(name)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            r.try_find_reference(name.as_bstr())
                .map(|value| value.map(|reference| Reference::from_native(handle, reference)))
                .map_err(to_py)
        })
    }
    fn references(&self) -> ReferenceIterPlatform {
        ReferenceIterPlatform {
            handle: self.handle.clone(),
        }
    }
    fn reference(
        &self,
        py: Python<'_>,
        name: &Bound<'_, PyAny>,
        target: &Bound<'_, PyAny>,
        constraint: PreviousValue,
        log_message: &Bound<'_, PyAny>,
    ) -> PyResult<Reference> {
        let name = full_name(name)?;
        let target = ObjectSpec::extract(target)?;
        let message = bytes(log_message)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            let target = target.resolve(r)?;
            let edit = gix::refs::transaction::RefEdit::update(
                name.clone(),
                target,
                constraint.inner.clone(),
                message.clone(),
            );
            validate_edit(r, &edit)?;
            r.reference(name, target, constraint.inner, message)
                .map(|reference| Reference::from_native(handle, reference))
                .map_err(to_py)
        })
    }
    fn tag_reference(
        &self,
        py: Python<'_>,
        name: String,
        target: &Bound<'_, PyAny>,
        constraint: PreviousValue,
    ) -> PyResult<Reference> {
        let target = ObjectSpec::extract(target)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |r| {
            let target = target.resolve(r)?;
            match &constraint.inner {
                gix::refs::transaction::PreviousValue::MustExistAndMatch(old)
                | gix::refs::transaction::PreviousValue::ExistingMustMatch(old) => validate_target(r, old)?,
                _ => {}
            }
            r.tag_reference(name, target, constraint.inner)
                .map(|reference| Reference::from_native(handle, reference))
                .map_err(to_py)
        })
    }
    fn edit_reference(&self, py: Python<'_>, edit: RefEdit) -> PyResult<Vec<RefEdit>> {
        self.edit_references(py, vec![edit])
    }
    fn edit_references(&self, py: Python<'_>, edits: Vec<RefEdit>) -> PyResult<Vec<RefEdit>> {
        self.handle.run(py, move |r| {
            for edit in &edits {
                validate_edit(r, &edit.inner)?;
            }
            r.edit_references(edits.into_iter().map(|edit| edit.inner))
                .map(|edits| edits.into_iter().map(|inner| RefEdit { inner }).collect())
                .map_err(to_py)
        })
    }
    #[pyo3(signature = (edits, committer=None))]
    fn edit_references_as(
        &self,
        py: Python<'_>,
        edits: Vec<RefEdit>,
        committer: Option<Signature>,
    ) -> PyResult<Vec<RefEdit>> {
        self.handle.run(py, move |r| {
            for edit in &edits {
                validate_edit(r, &edit.inner)?;
            }
            let mut time_buf = gix::date::parse::TimeBuf::default();
            r.edit_references_as(
                edits.into_iter().map(|edit| edit.inner),
                committer.as_ref().map(|s| s.inner.to_ref(&mut time_buf)),
            )
            .map(|edits| edits.into_iter().map(|inner| RefEdit { inner }).collect())
            .map_err(to_py)
        })
    }
    fn namespace<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let namespace = self
            .handle
            .run(py, |r| Ok(r.namespace().map(|n| n.as_bstr().to_vec())))?;
        Ok(namespace.map(|n| PyBytes::new(py, &n)))
    }
    fn set_namespace<'py>(
        &self,
        py: Python<'py>,
        namespace: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let name = bytes(namespace)?;
        let previous = self.handle.mutate(py, move |r| {
            r.set_namespace(name.as_bstr())
                .map(|n| n.map(|n| n.into_bstring()))
                .map_err(to_py)
        })?;
        Ok(previous.map(|n| PyBytes::new(py, &n)))
    }
    fn clear_namespace<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let previous = self
            .handle
            .mutate(py, |r| Ok(r.clear_namespace().map(|n| n.into_bstring())))?;
        Ok(previous.map(|n| PyBytes::new(py, &n)))
    }
    fn delete_local_branches(&self, py: Python<'_>, names: Vec<Bound<'_, PyAny>>) -> PyResult<Vec<Py<PyBytes>>> {
        let names = names.iter().map(full_name).collect::<PyResult<Vec<_>>>()?;
        let removed = self
            .handle
            .mutate(py, move |r| r.delete_local_branches(names).map_err(to_py))?;
        Ok(removed
            .into_iter()
            .map(|name| PyBytes::new(py, name.as_bstr()).unbind())
            .collect())
    }
    fn delete_local_branches_if_unchanged(
        &self,
        py: Python<'_>,
        branches: Vec<(Bound<'_, PyAny>, Target)>,
    ) -> PyResult<()> {
        let branches = branches
            .into_iter()
            .map(|(name, target)| full_name(&name).map(|name| (name, target.inner)))
            .collect::<PyResult<Vec<_>>>()?;
        self.handle.mutate(py, move |r| {
            for (_, target) in &branches {
                validate_target(r, target)?;
            }
            r.delete_local_branches_if_unchanged(branches).map_err(to_py)
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Target>()?;
    m.add_class::<PreviousValue>()?;
    m.add_class::<RefLog>()?;
    m.add_class::<LogChange>()?;
    m.add_class::<Change>()?;
    m.add_class::<RefEdit>()?;
    m.add_class::<Reference>()?;
    m.add_class::<Head>()?;
    m.add_class::<ReferenceIterPlatform>()?;
    m.add_class::<ReferenceIter>()?;
    m.add_class::<ReflogIterPlatform>()?;
    m.add_class::<ReflogLine>()?;
    m.add_class::<ReflogIter>()?;
    Ok(())
}
