use std::sync::Arc;

use crate::{
    error::to_py,
    objects::{Blob, Object},
    repository::Repository,
    runtime::CommandOwner,
    types::{ObjectId, ObjectSpec, bytes},
};
use gix::bstr::ByteSlice;
use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyBytes};

enum Command {
    Ready,
    WithRefs(Vec<Vec<u8>>),
    Message(String),
    DefaultRef,
    Ref(usize),
    Get(ObjectSpec),
    Replace(Vec<u8>, ObjectSpec, Vec<u8>, bool),
    Remove(Vec<u8>, ObjectSpec),
}
enum Reply {
    Unit,
    Name(Option<Vec<u8>>),
    Notes(Vec<Note>),
    Id(Option<ObjectId>),
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Notes {
    owner: Arc<CommandOwner<Command, Reply>>,
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct Note {
    reference: Vec<u8>,
    blob: Blob,
}
#[pymethods]
impl Note {
    #[getter]
    fn reference<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.reference)
    }
    #[getter]
    fn blob(&self) -> Blob {
        self.blob.clone()
    }
}

#[pymethods]
impl Notes {
    fn with_refs<'py>(slf: PyRef<'py, Self>, py: Python<'py>, refs: &Bound<'_, PyAny>) -> PyResult<PyRef<'py, Self>> {
        let refs = refs.try_iter()?.map(|v| bytes(&v?)).collect::<PyResult<Vec<_>>>()?;
        slf.owner.call(py, Command::WithRefs(refs))?;
        Ok(slf)
    }
    fn with_commit_message<'py>(slf: PyRef<'py, Self>, py: Python<'py>, message: String) -> PyResult<PyRef<'py, Self>> {
        slf.owner.call(py, Command::Message(message))?;
        Ok(slf)
    }
    fn default_ref<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        match self.owner.call(py, Command::DefaultRef)? {
            Reply::Name(v) => Ok(v.map(|v| PyBytes::new(py, &v))),
            _ => Err(unexpected()),
        }
    }
    fn refs(&self) -> NotesRefs {
        NotesRefs {
            notes: self.clone(),
            index: 0,
        }
    }
    fn get(&self, py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<Vec<Note>> {
        match self.owner.call(py, Command::Get(ObjectSpec::extract(object)?))? {
            Reply::Notes(v) => Ok(v),
            _ => Err(unexpected()),
        }
    }
    fn replace(
        &self,
        py: Python<'_>,
        notes_ref: &Bound<'_, PyAny>,
        object: &Bound<'_, PyAny>,
        data: &Bound<'_, PyAny>,
    ) -> PyResult<Option<ObjectId>> {
        match self.owner.call(
            py,
            Command::Replace(bytes(notes_ref)?, ObjectSpec::extract(object)?, bytes(data)?, false),
        )? {
            Reply::Id(v) => Ok(v),
            _ => Err(unexpected()),
        }
    }
    fn replace_at_ref(
        &self,
        py: Python<'_>,
        notes_ref: &Bound<'_, PyAny>,
        object: &Bound<'_, PyAny>,
        data: &Bound<'_, PyAny>,
    ) -> PyResult<Option<ObjectId>> {
        match self.owner.call(
            py,
            Command::Replace(bytes(notes_ref)?, ObjectSpec::extract(object)?, bytes(data)?, true),
        )? {
            Reply::Id(v) => Ok(v),
            _ => Err(unexpected()),
        }
    }
    fn remove(
        &self,
        py: Python<'_>,
        notes_ref: &Bound<'_, PyAny>,
        object: &Bound<'_, PyAny>,
    ) -> PyResult<Option<ObjectId>> {
        match self
            .owner
            .call(py, Command::Remove(bytes(notes_ref)?, ObjectSpec::extract(object)?))?
        {
            Reply::Id(v) => Ok(v),
            _ => Err(unexpected()),
        }
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
        _ty: &Bound<'_, PyAny>,
        _v: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}
fn unexpected() -> PyErr {
    PyRuntimeError::new_err("unexpected notes owner response")
}

#[pyclass(module = "gix")]
pub struct NotesRefs {
    notes: Notes,
    index: usize,
}
#[pymethods]
impl NotesRefs {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        match self.notes.owner.call(py, Command::Ref(self.index))? {
            Reply::Name(v) => {
                if v.is_some() {
                    self.index += 1;
                }
                Ok(v.map(|v| PyBytes::new(py, &v)))
            }
            _ => Err(unexpected()),
        }
    }
}

#[pymethods]
impl Repository {
    fn notes(&self, py: Python<'_>) -> PyResult<Notes> {
        let handle = self.handle.clone();
        let owner = CommandOwner::new("notes", move |_, commands, producer| {
            handle.with(|repo| {
                let mut native = Some(repo.notes().map_err(to_py)?);
                producer.serve(std::iter::from_fn(|| {
                    let command = commands.lock().unwrap_or_else(|e| e.into_inner()).take()?;
                    let result = (|| {
                        if let Command::WithRefs(refs) = command {
                            let current = native
                                .take()
                                .ok_or_else(|| PyRuntimeError::new_err("notes builder consumed"))?;
                            native = Some(current.with_refs(refs).map_err(to_py)?);
                            return Ok(Reply::Unit);
                        }
                        if let Command::Message(message) = command {
                            native = native.take().map(|n| n.with_commit_message(message));
                            return Ok(Reply::Unit);
                        }
                        let native = native
                            .as_mut()
                            .ok_or_else(|| PyRuntimeError::new_err("notes builder consumed"))?;
                        Ok(match command {
                            Command::Ready => Reply::Unit,
                            Command::DefaultRef => Reply::Name(native.default_ref().map(|v| v.as_bstr().to_vec())),
                            // ponytail: O(n²) across selected refs; native selection is usually a handful of refs.
                            Command::Ref(index) => Reply::Name(native.refs().nth(index).map(|v| v.as_bstr().to_vec())),
                            Command::Get(spec) => Reply::Notes(
                                native
                                    .get(spec.resolve(repo)?)
                                    .map_err(to_py)?
                                    .into_iter()
                                    .map(|note| Note {
                                        reference: note.reference.as_bstr().to_vec(),
                                        blob: Blob {
                                            object: Object::from_detached(handle.clone(), note.blob.detach()),
                                        },
                                    })
                                    .collect(),
                            ),
                            Command::Replace(name, spec, data, full) => {
                                let id = spec.resolve(repo)?;
                                let previous = if full {
                                    let name = gix::refs::FullName::try_from(name.as_bstr()).map_err(to_py)?;
                                    native.replace_at_ref(name.as_ref(), id, data)
                                } else {
                                    native.replace(gix::bstr::BString::from(name), id, data)
                                }
                                .map_err(to_py)?;
                                Reply::Id(previous.map(|v| ObjectId { inner: v.detach() }))
                            }
                            Command::Remove(name, spec) => Reply::Id(
                                native
                                    .remove(gix::bstr::BString::from(name), spec.resolve(repo)?)
                                    .map_err(to_py)?
                                    .map(|v| ObjectId { inner: v.detach() }),
                            ),
                            Command::WithRefs(_) | Command::Message(_) => unreachable!("handled above"),
                        })
                    })();
                    Some(Ok(result))
                }))
            })
        });
        owner.call(py, Command::Ready)?;
        Ok(Notes { owner: Arc::new(owner) })
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Notes>()?;
    m.add_class::<Note>()?;
    m.add_class::<NotesRefs>()?;
    Ok(())
}
