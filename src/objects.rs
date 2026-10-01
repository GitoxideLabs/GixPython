//! Owned Git objects and lazy views over their native encodings.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};

use crate::{
    error::to_py,
    references::{PreviousValue, Reference},
    repository::{RepoHandle, Repository},
    runtime::{CancellationToken, OwnedIter, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Copy)]
pub struct Time {
    #[pyo3(get)]
    pub seconds: i64,
    #[pyo3(get)]
    pub offset: i32,
}

impl From<gix::date::Time> for Time {
    fn from(value: gix::date::Time) -> Self {
        Self {
            seconds: value.seconds,
            offset: value.offset,
        }
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Signature {
    pub inner: gix::actor::Signature,
}

#[pymethods]
impl Signature {
    #[new]
    #[pyo3(signature = (name, email, seconds, offset=0))]
    fn new(name: &Bound<'_, PyAny>, email: &Bound<'_, PyAny>, seconds: i64, offset: i32) -> PyResult<Self> {
        let inner = gix::actor::Signature {
            name: bytes(name)?.into(),
            email: bytes(email)?.into(),
            time: gix::date::Time { seconds, offset },
        };
        inner.write_to(&mut std::io::sink()).map_err(to_py)?;
        Ok(Self { inner })
    }

    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.name.as_ref())
    }

    #[getter]
    fn email<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.email.as_ref())
    }

    #[getter]
    fn time(&self) -> Time {
        self.inner.time.into()
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Object {
    pub handle: RepoHandle,
    pub inner: Arc<gix::ObjectDetached>,
}

impl Object {
    pub fn from_native(handle: RepoHandle, object: gix::Object<'_>) -> Self {
        Self::from_detached(handle, object.detach())
    }

    pub fn from_detached(handle: RepoHandle, object: gix::ObjectDetached) -> Self {
        Self {
            handle,
            inner: Arc::new(object),
        }
    }

    fn require_kind(&self, kind: gix::objs::Kind) -> PyResult<()> {
        if self.inner.kind != kind {
            return Err(PyValueError::new_err(format!(
                "expected {kind}, got {}",
                self.inner.kind
            )));
        }
        Ok(())
    }

    fn peel(&self, py: Python<'_>, target: Option<gix::objs::Kind>) -> PyResult<Self> {
        let this = self.clone();
        self.handle.run(py, move |repo| {
            let mut object = this.inner.as_ref().clone();
            loop {
                if target == Some(object.kind) || (target.is_none() && object.kind != gix::objs::Kind::Tag) {
                    return Ok(Self::from_detached(this.handle.clone(), object));
                }
                // The higher-level native peel helpers panic on malformed intermediate
                // objects. Use their native fallible token readers to preserve errors.
                let id = match object.kind {
                    gix::objs::Kind::Commit => gix::objs::CommitRefIter::from_bytes(&object.data, object.id.kind())
                        .tree_id()
                        .map_err(to_py)?,
                    gix::objs::Kind::Tag => gix::objs::TagRefIter::from_bytes(&object.data, object.id.kind())
                        .target_id()
                        .map_err(to_py)?,
                    _ => return Err(PyValueError::new_err("object cannot be peeled to the requested kind")),
                };
                object = repo.find_object(id).map_err(to_py)?.detach();
            }
        })
    }
}

#[pymethods]
impl Object {
    #[getter]
    fn id(&self) -> ObjectId {
        ObjectId { inner: self.inner.id }
    }

    #[getter]
    fn kind(&self) -> String {
        self.inner.kind.to_string()
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.data)
    }

    fn __repr__(&self) -> String {
        format!("Object({}, '{}')", self.inner.kind, self.inner.id)
    }

    fn try_into_blob(&self) -> PyResult<Blob> {
        self.require_kind(gix::objs::Kind::Blob)?;
        Ok(Blob { object: self.clone() })
    }

    fn try_into_tree(&self) -> PyResult<Tree> {
        self.require_kind(gix::objs::Kind::Tree)?;
        Ok(Tree { object: self.clone() })
    }

    fn try_into_commit(&self) -> PyResult<Commit> {
        self.require_kind(gix::objs::Kind::Commit)?;
        Ok(Commit { object: self.clone() })
    }

    fn try_into_tag(&self) -> PyResult<Tag> {
        self.require_kind(gix::objs::Kind::Tag)?;
        Ok(Tag { object: self.clone() })
    }

    fn into_blob(&self) -> PyResult<Blob> {
        self.try_into_blob()
    }
    fn into_tree(&self) -> PyResult<Tree> {
        self.try_into_tree()
    }
    fn into_commit(&self) -> PyResult<Commit> {
        self.try_into_commit()
    }
    fn into_tag(&self) -> PyResult<Tag> {
        self.try_into_tag()
    }

    fn peel_to_kind(&self, py: Python<'_>, kind: &str) -> PyResult<Self> {
        self.peel(py, Some(parse_kind(kind)?))
    }

    fn peel_to_tree(&self, py: Python<'_>) -> PyResult<Tree> {
        self.peel(py, Some(gix::objs::Kind::Tree))?.try_into_tree()
    }

    fn peel_to_commit(&self, py: Python<'_>) -> PyResult<Commit> {
        self.peel(py, Some(gix::objs::Kind::Commit))?.try_into_commit()
    }

    fn peel_tags_to_end(&self, py: Python<'_>) -> PyResult<Self> {
        self.peel(py, None)
    }
}

macro_rules! typed_object {
    ($name:ident, $kind:literal) => {
        #[pyclass(frozen, module = "gix", from_py_object)]
        #[derive(Clone)]
        pub struct $name {
            pub object: Object,
        }

        #[pymethods]
        impl $name {
            #[getter]
            fn id(&self) -> ObjectId {
                self.object.id()
            }
            #[getter]
            fn kind(&self) -> &'static str {
                $kind
            }
            #[getter]
            fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                self.object.data(py)
            }
            fn __repr__(&self) -> String {
                format!("{}('{}')", stringify!($name), self.object.inner.id)
            }
            fn into_object(&self) -> Object {
                self.object.clone()
            }
        }
    };
}

typed_object!(Blob, "blob");
typed_object!(Tree, "tree");
typed_object!(Commit, "commit");
typed_object!(Tag, "tag");

#[pyclass(frozen, module = "gix")]
pub struct Header {
    inner: gix::odb::find::Header,
}

#[pymethods]
impl Header {
    fn kind(&self) -> String {
        self.inner.kind().to_string()
    }
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn num_deltas(&self) -> Option<u32> {
        self.inner.num_deltas()
    }
}

/// Owned copies of the raw fields in a native CommitRef. Decoding never rewrites
/// headers or requires signature timestamps to parse successfully.
#[pyclass(frozen, module = "gix")]
pub struct CommitData {
    tree: Vec<u8>,
    parents: Vec<Vec<u8>>,
    author: Vec<u8>,
    committer: Vec<u8>,
    encoding: Option<Vec<u8>>,
    message: Vec<u8>,
    extra_headers: Vec<(Vec<u8>, Vec<u8>)>,
}

#[pymethods]
impl CommitData {
    #[getter]
    fn tree<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.tree)
    }
    #[getter]
    fn parents<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
        pyo3::types::PyTuple::new(py, self.parents.iter().map(|id| PyBytes::new(py, id)))
    }
    #[getter]
    fn author<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.author)
    }
    #[getter]
    fn committer<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.committer)
    }
    #[getter]
    fn encoding<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.encoding.as_ref().map(|v| PyBytes::new(py, v))
    }
    #[getter]
    fn message<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.message)
    }
    #[getter]
    fn extra_headers<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
        pyo3::types::PyTuple::new(
            py,
            self.extra_headers
                .iter()
                .map(|(name, value)| (PyBytes::new(py, name), PyBytes::new(py, value))),
        )
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TagData {
    target: Vec<u8>,
    #[pyo3(get)]
    target_kind: String,
    name: Vec<u8>,
    tagger: Option<Vec<u8>>,
    message: Vec<u8>,
    signature: Option<Vec<u8>>,
}

#[pymethods]
impl TagData {
    #[getter]
    fn target<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.target)
    }
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }
    #[getter]
    fn tagger<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.tagger.as_ref().map(|v| PyBytes::new(py, v))
    }
    #[getter]
    fn message<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.message)
    }
    #[getter]
    fn signature<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.signature.as_ref().map(|v| PyBytes::new(py, v))
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeData {
    handle: RepoHandle,
    entries: Vec<gix::objs::tree::Entry>,
}

#[pymethods]
impl TreeData {
    #[getter]
    fn entries<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
        pyo3::types::PyTuple::new(
            py,
            self.entries.iter().map(|entry| TreeEntry {
                handle: self.handle.clone(),
                inner: entry.clone(),
            }),
        )
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeEntry {
    pub handle: RepoHandle,
    pub inner: gix::objs::tree::Entry,
}

#[pymethods]
impl TreeEntry {
    fn filename<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.filename.as_ref())
    }
    fn mode(&self) -> u32 {
        self.inner.mode.value() as u32
    }
    fn kind(&self) -> String {
        self.inner.mode.as_str().to_owned()
    }
    fn id(&self) -> ObjectId {
        ObjectId { inner: self.inner.oid }
    }
    fn object_id(&self) -> ObjectId {
        self.id()
    }
    fn object(&self, py: Python<'_>) -> PyResult<Object> {
        let handle = self.handle.clone();
        let id = self.inner.oid;
        self.handle.run(py, move |repo| {
            Ok(Object::from_native(handle, repo.find_object(id).map_err(to_py)?))
        })
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeEntries {
    inner: OwnedIter<TreeEntry, gix::Error>,
}

#[pyclass(frozen, module = "gix")]
pub struct ParentIds {
    inner: OwnedIter<ObjectId, gix::Error>,
}

macro_rules! native_iterator {
    ($name:ident, $item:ty) => {
        #[pymethods]
        impl $name {
            fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                slf
            }
            fn __next__(&self, py: Python<'_>) -> PyResult<Option<$item>> {
                self.inner.next(py)?.transpose().map_err(to_py)
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
                _ty: &Bound<'_, PyAny>,
                _value: &Bound<'_, PyAny>,
                _tb: &Bound<'_, PyAny>,
            ) -> PyResult<()> {
                self.inner.close(py)
            }
        }
    };
}

native_iterator!(TreeEntries, TreeEntry);
native_iterator!(ParentIds, ObjectId);

enum EditorCommand {
    Ready,
    Upsert(Vec<u8>, gix::objs::tree::EntryKind, ObjectSpec),
    Remove(Vec<u8>, bool),
    Get(Vec<u8>),
    SetRoot(Arc<gix::ObjectDetached>),
    Write,
}

enum EditorReply {
    Unit,
    Entry(Option<TreeEntry>),
    Id(ObjectId),
}

enum EditorRoot {
    Id(ObjectSpec),
    Object(Arc<gix::ObjectDetached>),
}

/// One native editor and repository live together on their owning worker's stack.
/// Commands use the same demand-driven transport as native iterators, so no borrowed
/// Rust value escapes or requires a self-referential allocation.
#[pyclass(frozen, module = "gix")]
pub struct TreeEditor {
    commands: Arc<Mutex<Option<EditorCommand>>>,
    executing: AtomicBool,
    inner: OwnedIter<PyResult<EditorReply>, PyErr>,
}

impl TreeEditor {
    fn new(py: Python<'_>, handle: RepoHandle, tree: EditorRoot) -> PyResult<Self> {
        let commands = Arc::new(Mutex::new(None));
        let incoming = commands.clone();
        let inner = OwnedIter::new("tree editor", None, None, move |_, producer| {
            handle.with(|repo| {
                let root = match tree {
                    EditorRoot::Id(id) => repo.find_tree(id.resolve(repo)?).map_err(to_py)?,
                    EditorRoot::Object(object) => gix::Tree::from_data(object.id, object.data.clone(), repo),
                };
                let mut editor = root.edit().map_err(to_py)?;
                producer.serve(std::iter::from_fn(|| {
                    let command = incoming.lock().unwrap_or_else(|e| e.into_inner()).take()?;
                    let result = (|| match command {
                        EditorCommand::Ready => Ok(EditorReply::Unit),
                        EditorCommand::Upsert(path, kind, id) => {
                            editor.upsert(path.as_bstr(), kind, id.resolve(repo)?).map_err(to_py)?;
                            Ok(EditorReply::Unit)
                        }
                        EditorCommand::Remove(path, leaf) => {
                            if leaf {
                                editor.remove_leaf(path.as_bstr())
                            } else {
                                editor.remove(path.as_bstr())
                            }
                            .map_err(to_py)?;
                            Ok(EditorReply::Unit)
                        }
                        EditorCommand::Get(path) => {
                            Ok(EditorReply::Entry(editor.get(path.as_bstr()).map(|entry| TreeEntry {
                                handle: handle.clone(),
                                inner: entry.detach().into(),
                            })))
                        }
                        EditorCommand::SetRoot(object) => {
                            if object.id.kind() != repo.object_hash() {
                                return Err(PyValueError::new_err("tree hash kind differs from the repository"));
                            }
                            let tree = gix::Tree::from_data(object.id, object.data.clone(), repo);
                            editor.set_root(&tree).map_err(to_py)?;
                            Ok(EditorReply::Unit)
                        }
                        EditorCommand::Write => editor
                            .write()
                            .map(|id| EditorReply::Id(ObjectId { inner: id.detach() }))
                            .map_err(to_py),
                    })();
                    Some(Ok(result))
                }))
            })
        });
        let editor = Self {
            commands,
            executing: AtomicBool::new(false),
            inner,
        };
        editor.call(py, EditorCommand::Ready)?;
        Ok(editor)
    }

    fn call(&self, py: Python<'_>, command: EditorCommand) -> PyResult<EditorReply> {
        if self
            .executing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(PyRuntimeError::new_err("tree editor is already executing"));
        }
        struct Release<'a>(&'a AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _release = Release(&self.executing);
        *self.commands.lock().unwrap_or_else(|e| e.into_inner()) = Some(command);
        self.inner
            .next(py)?
            .ok_or_else(|| PyRuntimeError::new_err("tree editor is closed"))??
    }
}

#[pymethods]
impl TreeEditor {
    fn upsert<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        rela_path: &Bound<'py, PyAny>,
        kind: &str,
        id: &Bound<'py, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        use gix::objs::tree::EntryKind;
        let kind = match kind {
            "tree" => EntryKind::Tree,
            "blob" => EntryKind::Blob,
            "exe" => EntryKind::BlobExecutable,
            "link" => EntryKind::Link,
            "commit" => EntryKind::Commit,
            _ => return Err(PyValueError::new_err("kind must be tree, blob, exe, link, or commit")),
        };
        slf.call(
            py,
            EditorCommand::Upsert(bytes(rela_path)?, kind, ObjectSpec::extract(id)?),
        )?;
        Ok(slf)
    }
    fn remove<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        rela_path: &Bound<'py, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        slf.call(py, EditorCommand::Remove(bytes(rela_path)?, false))?;
        Ok(slf)
    }
    fn remove_leaf<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        rela_path: &Bound<'py, PyAny>,
    ) -> PyResult<PyRef<'py, Self>> {
        slf.call(py, EditorCommand::Remove(bytes(rela_path)?, true))?;
        Ok(slf)
    }
    fn get(&self, py: Python<'_>, rela_path: &Bound<'_, PyAny>) -> PyResult<Option<TreeEntry>> {
        match self.call(py, EditorCommand::Get(bytes(rela_path)?))? {
            EditorReply::Entry(entry) => Ok(entry),
            _ => Err(PyRuntimeError::new_err("unexpected tree editor response")),
        }
    }
    fn set_root<'py>(slf: PyRef<'py, Self>, py: Python<'py>, root: &Tree) -> PyResult<PyRef<'py, Self>> {
        slf.call(py, EditorCommand::SetRoot(root.object.inner.clone()))?;
        Ok(slf)
    }
    fn write(&self, py: Python<'_>) -> PyResult<ObjectId> {
        match self.call(py, EditorCommand::Write)? {
            EditorReply::Id(id) => Ok(id),
            _ => Err(PyRuntimeError::new_err("unexpected tree editor response")),
        }
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
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.inner.close(py)
    }
}

#[pymethods]
impl Tree {
    fn edit(&self, py: Python<'_>) -> PyResult<TreeEditor> {
        TreeEditor::new(
            py,
            self.object.handle.clone(),
            EditorRoot::Object(self.object.inner.clone()),
        )
    }

    fn decode(&self, py: Python<'_>) -> PyResult<TreeData> {
        let object = self.object.clone();
        py.detach(move || {
            let tree = gix::objs::TreeRef::from_bytes(&object.inner.data, object.inner.id.kind()).map_err(to_py)?;
            Ok(TreeData {
                handle: object.handle,
                entries: tree.entries.into_iter().map(Into::into).collect(),
            })
        })
    }

    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn iter(&self, progress: Option<&Progress>, cancel: Option<&CancellationToken>) -> TreeEntries {
        let object = self.object.clone();
        TreeEntries {
            inner: OwnedIter::new("tree entries", progress, cancel, move |_, producer| {
                producer.serve(
                    gix::objs::TreeRefIter::from_bytes(&object.inner.data, object.inner.id.kind()).map(|entry| {
                        entry.map(|entry| TreeEntry {
                            handle: object.handle.clone(),
                            inner: entry.into(),
                        })
                    }),
                )
            }),
        }
    }

    fn __iter__(&self) -> TreeEntries {
        self.iter(None, None)
    }

    fn find_entry(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Option<TreeEntry>> {
        let name = bytes(name)?;
        let object = self.object.clone();
        py.detach(move || {
            for entry in gix::objs::TreeRefIter::from_bytes(&object.inner.data, object.inner.id.kind()) {
                let entry = entry.map_err(to_py)?;
                if entry.filename == name.as_bstr() {
                    return Ok(Some(TreeEntry {
                        handle: object.handle.clone(),
                        inner: entry.into(),
                    }));
                }
            }
            Ok(None)
        })
    }

    fn lookup_entry(&self, py: Python<'_>, path: Vec<Bound<'_, PyAny>>) -> PyResult<Option<TreeEntry>> {
        let path = path.iter().map(bytes).collect::<PyResult<Vec<_>>>()?;
        let object = self.object.clone();
        self.object.handle.run(py, move |repo| {
            let tree = gix::Tree::from_data(object.inner.id, object.inner.data.clone(), repo);
            let entry = tree.lookup_entry(path.iter().map(Vec::as_slice)).map_err(to_py)?;
            Ok(entry.map(|entry| TreeEntry {
                handle: object.handle.clone(),
                inner: entry.detach(),
            }))
        })
    }

    fn lookup_entry_by_path(&self, py: Python<'_>, relative_path: std::path::PathBuf) -> PyResult<Option<TreeEntry>> {
        let object = self.object.clone();
        self.object.handle.run(py, move |repo| {
            let tree = gix::Tree::from_data(object.inner.id, object.inner.data.clone(), repo);
            Ok(tree
                .lookup_entry_by_path(relative_path)
                .map_err(to_py)?
                .map(|entry| TreeEntry {
                    handle: object.handle.clone(),
                    inner: entry.detach(),
                }))
        })
    }
}

#[pymethods]
impl Commit {
    fn decode(&self, py: Python<'_>) -> PyResult<CommitData> {
        let object = self.object.inner.clone();
        py.detach(move || {
            let commit = gix::objs::CommitRef::from_bytes(&object.data, object.id.kind()).map_err(to_py)?;
            Ok(CommitData {
                tree: commit.tree.to_vec(),
                parents: commit.parents.iter().map(|v| v.to_vec()).collect(),
                author: commit.author.to_vec(),
                committer: commit.committer.to_vec(),
                encoding: commit.encoding.map(|v| v.to_vec()),
                message: commit.message.to_vec(),
                extra_headers: commit
                    .extra_headers
                    .iter()
                    .map(|(k, v)| (k.to_vec(), v.to_vec()))
                    .collect(),
            })
        })
    }

    fn message_raw<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let object = self.object.inner.clone();
        let message = py.detach(move || {
            gix::objs::CommitRefIter::from_bytes(&object.data, object.id.kind())
                .message()
                .map(|message| message.to_vec())
                .map_err(to_py)
        })?;
        Ok(PyBytes::new(py, &message))
    }

    fn author(&self, py: Python<'_>) -> PyResult<Signature> {
        let object = self.object.inner.clone();
        py.detach(move || {
            gix::objs::CommitRefIter::from_bytes(&object.data, object.id.kind())
                .author()
                .and_then(|signature| signature.trim().to_owned())
                .map(|inner| Signature { inner })
                .map_err(to_py)
        })
    }

    fn committer(&self, py: Python<'_>) -> PyResult<Signature> {
        let object = self.object.inner.clone();
        py.detach(move || {
            gix::objs::CommitRefIter::from_bytes(&object.data, object.id.kind())
                .committer()
                .and_then(|signature| signature.trim().to_owned())
                .map(|inner| Signature { inner })
                .map_err(to_py)
        })
    }

    fn time(&self, py: Python<'_>) -> PyResult<Time> {
        Ok(self.committer(py)?.inner.time.into())
    }

    fn tree_id(&self, py: Python<'_>) -> PyResult<ObjectId> {
        let object = self.object.inner.clone();
        py.detach(move || {
            gix::objs::CommitRefIter::from_bytes(&object.data, object.id.kind())
                .tree_id()
                .map(|inner| ObjectId { inner })
                .map_err(to_py)
        })
    }

    fn tree(&self, py: Python<'_>) -> PyResult<Tree> {
        let id = self.tree_id(py)?.inner;
        let handle = self.object.handle.clone();
        self.object.handle.run(py, move |repo| {
            Ok(Tree {
                object: Object::from_detached(handle, repo.find_tree(id).map_err(to_py)?.detach()),
            })
        })
    }

    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn parent_ids(&self, progress: Option<&Progress>, cancel: Option<&CancellationToken>) -> ParentIds {
        let object = self.object.inner.clone();
        ParentIds {
            inner: OwnedIter::new("commit parents", progress, cancel, move |_, producer| {
                let iter =
                    gix::objs::CommitRefIter::from_bytes(&object.data, object.id.kind()).filter_map(
                        |token| match token {
                            Ok(gix::objs::commit::ref_iter::Token::Parent { id }) => Some(Ok(ObjectId { inner: id })),
                            Err(error) => Some(Err(error)),
                            _ => None,
                        },
                    );
                producer.serve(iter)
            }),
        }
    }
}

#[pymethods]
impl Tag {
    fn decode(&self, py: Python<'_>) -> PyResult<TagData> {
        let object = self.object.inner.clone();
        py.detach(move || {
            let tag = gix::objs::TagRef::from_bytes(&object.data, object.id.kind()).map_err(to_py)?;
            Ok(TagData {
                target: tag.target.to_vec(),
                target_kind: tag.target_kind.to_string(),
                name: tag.name.to_vec(),
                tagger: tag.tagger.map(|v| v.to_vec()),
                message: tag.message.to_vec(),
                signature: tag.signature.map(|v| v.to_vec()),
            })
        })
    }

    fn target_id(&self, py: Python<'_>) -> PyResult<ObjectId> {
        let object = self.object.inner.clone();
        py.detach(move || {
            gix::objs::TagRefIter::from_bytes(&object.data, object.id.kind())
                .target_id()
                .map(|inner| ObjectId { inner })
                .map_err(to_py)
        })
    }

    fn tagger(&self, py: Python<'_>) -> PyResult<Option<Signature>> {
        let object = self.object.inner.clone();
        py.detach(move || {
            gix::objs::TagRefIter::from_bytes(&object.data, object.id.kind())
                .tagger()
                .and_then(|signature| signature.map(|s| s.to_owned()).transpose())
                .map(|signature| signature.map(|inner| Signature { inner }))
                .map_err(to_py)
        })
    }
}

fn parse_kind(kind: &str) -> PyResult<gix::objs::Kind> {
    match kind {
        "blob" => Ok(gix::objs::Kind::Blob),
        "tree" => Ok(gix::objs::Kind::Tree),
        "commit" => Ok(gix::objs::Kind::Commit),
        "tag" => Ok(gix::objs::Kind::Tag),
        _ => Err(PyValueError::new_err("kind must be blob, tree, commit, or tag")),
    }
}

pub fn object_id(value: &Bound<'_, PyAny>) -> Option<gix::ObjectId> {
    macro_rules! typed {
        ($($name:ty),*) => { $(if let Ok(value) = value.extract::<PyRef<'_, $name>>() { return Some(value.object.inner.id); })* };
    }
    if let Ok(value) = value.extract::<PyRef<'_, Object>>() {
        return Some(value.inner.id);
    }
    typed!(Blob, Tree, Commit, Tag);
    None
}

#[pymethods]
impl Repository {
    fn edit_tree(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<TreeEditor> {
        TreeEditor::new(py, self.handle.clone(), EditorRoot::Id(ObjectSpec::extract(id)?))
    }

    fn find_header(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Header> {
        let id = ObjectSpec::extract(id)?;
        self.handle.run(py, move |repo| {
            repo.find_header(id.resolve(repo)?)
                .map(|inner| Header { inner })
                .map_err(to_py)
        })
    }

    fn try_find_header(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Option<Header>> {
        let id = ObjectSpec::extract(id)?;
        self.handle.run(py, move |repo| {
            repo.try_find_header(id.resolve(repo)?)
                .map(|header| header.map(|inner| Header { inner }))
                .map_err(to_py)
        })
    }

    fn find_object(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Object> {
        let id = ObjectSpec::extract(id)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            Ok(Object::from_native(
                handle,
                repo.find_object(id.resolve(repo)?).map_err(to_py)?,
            ))
        })
    }

    fn try_find_object(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Option<Object>> {
        let id = ObjectSpec::extract(id)?;
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            Ok(repo
                .try_find_object(id.resolve(repo)?)
                .map_err(to_py)?
                .map(|object| Object::from_native(handle, object)))
        })
    }

    fn find_blob(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Blob> {
        self.find_object(py, id)?.try_into_blob()
    }
    fn find_tree(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Tree> {
        self.find_object(py, id)?.try_into_tree()
    }
    fn find_commit(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Commit> {
        self.find_object(py, id)?.try_into_commit()
    }
    fn find_tag(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Tag> {
        self.find_object(py, id)?.try_into_tag()
    }

    fn has_object(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<bool> {
        let id = ObjectSpec::extract(id)?;
        self.handle.run(py, move |repo| Ok(repo.has_object(id.resolve(repo)?)))
    }

    fn write_blob(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<ObjectId> {
        let data = bytes(data)?;
        self.handle.run(py, move |repo| {
            repo.write_blob(data)
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }

    /// Write a validated native object from its kind and encoded bytes.
    fn write_object(&self, py: Python<'_>, kind: &str, data: &Bound<'_, PyAny>) -> PyResult<ObjectId> {
        let kind = parse_kind(kind)?;
        let data = bytes(data)?;
        self.handle.run(py, move |repo| {
            let object = gix::objs::Data::new(&data, kind, repo.object_hash())
                .decode()
                .map_err(to_py)?;
            repo.write_object(object)
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }

    /// gix itself buffers a complete input stream when writing a blob.
    fn write_blob_stream(&self, py: Python<'_>, stream: &Bound<'_, PyAny>) -> PyResult<ObjectId> {
        let data = stream.call_method0("read")?;
        self.write_blob(py, &data)
    }

    fn empty_tree(&self, py: Python<'_>) -> PyResult<Tree> {
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            Ok(Tree {
                object: Object::from_detached(handle, repo.empty_tree().detach()),
            })
        })
    }

    fn empty_blob(&self, py: Python<'_>) -> PyResult<Blob> {
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            Ok(Blob {
                object: Object::from_detached(handle, repo.empty_blob().detach()),
            })
        })
    }

    #[pyo3(signature = (name, target, target_kind, tagger, message, constraint))]
    fn tag(
        &self,
        py: Python<'_>,
        name: String,
        target: &Bound<'_, PyAny>,
        target_kind: &str,
        tagger: Option<&Signature>,
        message: String,
        constraint: PreviousValue,
    ) -> PyResult<Reference> {
        let target = ObjectSpec::extract(target)?;
        let kind = parse_kind(target_kind)?;
        let tagger = tagger.map(|v| v.inner.clone());
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            match &constraint.inner {
                gix::refs::transaction::PreviousValue::MustExistAndMatch(old)
                | gix::refs::transaction::PreviousValue::ExistingMustMatch(old) => {
                    crate::references::validate_target(repo, old)?
                }
                _ => {}
            }
            let mut buf = Default::default();
            repo.tag(
                name,
                target.resolve(repo)?,
                kind,
                tagger.as_ref().map(|v| v.to_ref(&mut buf)),
                message,
                constraint.inner,
            )
            .map(|reference| Reference::from_native(handle, reference))
            .map_err(to_py)
        })
    }

    #[pyo3(signature = (reference, message, tree, parents=Vec::new()))]
    fn commit(
        &self,
        py: Python<'_>,
        reference: &Bound<'_, PyAny>,
        message: &str,
        tree: &Bound<'_, PyAny>,
        parents: Vec<Bound<'_, PyAny>>,
    ) -> PyResult<ObjectId> {
        let reference = bytes(reference)?;
        let message = message.to_owned();
        let tree = ObjectSpec::extract(tree)?;
        let parents = parents.iter().map(ObjectSpec::extract).collect::<PyResult<Vec<_>>>()?;
        self.handle.run(py, move |repo| {
            let parents = parents
                .iter()
                .map(|id| id.resolve(repo))
                .collect::<PyResult<Vec<_>>>()?;
            repo.commit(reference.as_bstr(), message, tree.resolve(repo)?, parents)
                .map(|id| ObjectId { inner: id.detach() })
                .map_err(to_py)
        })
    }

    #[pyo3(signature = (committer, author, reference, message, tree, parents=Vec::new()))]
    #[allow(clippy::too_many_arguments)]
    fn commit_as(
        &self,
        py: Python<'_>,
        committer: &Signature,
        author: &Signature,
        reference: &Bound<'_, PyAny>,
        message: &str,
        tree: &Bound<'_, PyAny>,
        parents: Vec<Bound<'_, PyAny>>,
    ) -> PyResult<ObjectId> {
        let committer = committer.inner.clone();
        let author = author.inner.clone();
        let reference = bytes(reference)?;
        let message = message.to_owned();
        let tree = ObjectSpec::extract(tree)?;
        let parents = parents.iter().map(ObjectSpec::extract).collect::<PyResult<Vec<_>>>()?;
        self.handle.run(py, move |repo| {
            let parents = parents
                .iter()
                .map(|id| id.resolve(repo))
                .collect::<PyResult<Vec<_>>>()?;
            repo.commit_as(
                committer.to_ref(&mut Default::default()),
                author.to_ref(&mut Default::default()),
                reference.as_bstr(),
                message,
                tree.resolve(repo)?,
                parents,
            )
            .map(|id| ObjectId { inner: id.detach() })
            .map_err(to_py)
        })
    }

    #[pyo3(signature = (message, tree, parents=Vec::new()))]
    fn new_commit(
        &self,
        py: Python<'_>,
        message: &str,
        tree: &Bound<'_, PyAny>,
        parents: Vec<Bound<'_, PyAny>>,
    ) -> PyResult<Commit> {
        let message = message.to_owned();
        let tree = ObjectSpec::extract(tree)?;
        let parents = parents.iter().map(ObjectSpec::extract).collect::<PyResult<Vec<_>>>()?;
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            let parents = parents
                .iter()
                .map(|id| id.resolve(repo))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(Commit {
                object: Object::from_detached(
                    handle,
                    repo.new_commit(message, tree.resolve(repo)?, parents)
                        .map_err(to_py)?
                        .detach(),
                ),
            })
        })
    }

    #[pyo3(signature = (committer, author, message, tree, parents=Vec::new()))]
    fn new_commit_as(
        &self,
        py: Python<'_>,
        committer: &Signature,
        author: &Signature,
        message: &str,
        tree: &Bound<'_, PyAny>,
        parents: Vec<Bound<'_, PyAny>>,
    ) -> PyResult<Commit> {
        let committer = committer.inner.clone();
        let author = author.inner.clone();
        let message = message.to_owned();
        let tree = ObjectSpec::extract(tree)?;
        let parents = parents.iter().map(ObjectSpec::extract).collect::<PyResult<Vec<_>>>()?;
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            let parents = parents
                .iter()
                .map(|id| id.resolve(repo))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(Commit {
                object: Object::from_detached(
                    handle,
                    repo.new_commit_as(
                        committer.to_ref(&mut Default::default()),
                        author.to_ref(&mut Default::default()),
                        message,
                        tree.resolve(repo)?,
                        parents,
                    )
                    .map_err(to_py)?
                    .detach(),
                ),
            })
        })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Object>()?;
    m.add_class::<Blob>()?;
    m.add_class::<Tree>()?;
    m.add_class::<Commit>()?;
    m.add_class::<Tag>()?;
    m.add_class::<TreeEntry>()?;
    m.add_class::<TreeEntries>()?;
    m.add_class::<ParentIds>()?;
    m.add_class::<Signature>()?;
    m.add_class::<Time>()?;
    m.add_class::<Header>()?;
    m.add_class::<CommitData>()?;
    m.add_class::<TagData>()?;
    m.add_class::<TreeData>()?;
    m.add_class::<TreeEditor>()?;
    Ok(())
}
