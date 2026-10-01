use std::path::PathBuf;

use gix::bstr::ByteSlice;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};

use crate::{
    error::to_py,
    objects::{Commit, Object},
    repository::{RepoHandle, Repository},
    runtime::{CancellationToken, OwnedIter, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};

fn specs(values: &Bound<'_, PyAny>) -> PyResult<Vec<ObjectSpec>> {
    values.try_iter()?.map(|v| ObjectSpec::extract(&v?)).collect()
}
fn resolve(values: &[ObjectSpec], repo: &gix::Repository) -> PyResult<Vec<gix::ObjectId>> {
    values.iter().map(|v| v.resolve(repo)).collect()
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RevisionSpec {
    kind: String,
    display: String,
    single: Option<ObjectId>,
    path: Option<(Vec<u8>, u16)>,
}
#[pymethods]
impl RevisionSpec {
    fn kind(&self) -> &str {
        &self.kind
    }
    fn single(&self) -> Option<ObjectId> {
        self.single
    }
    fn path_and_mode<'py>(&self, py: Python<'py>) -> Option<(Bound<'py, PyBytes>, u16)> {
        self.path.as_ref().map(|(p, m)| (PyBytes::new(py, p), *m))
    }
    fn __str__(&self) -> &str {
        &self.display
    }
}

#[derive(Clone)]
enum WalkChange {
    Sorting(gix::revision::walk::Sorting),
    FirstParent,
    CommitGraph(Option<bool>),
    Boundary(Vec<ObjectSpec>),
    Hidden(Vec<ObjectSpec>),
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RevisionWalkPlatform {
    handle: RepoHandle,
    tips: Vec<ObjectSpec>,
    changes: Vec<WalkChange>,
}

impl RevisionWalkPlatform {
    pub fn new(handle: RepoHandle, tips: Vec<ObjectSpec>) -> Self {
        Self {
            handle,
            tips,
            changes: Vec::new(),
        }
    }
    fn changed(&self, change: WalkChange) -> Self {
        let mut out = self.clone();
        out.changes.push(change);
        out
    }
}
#[pymethods]
impl RevisionWalkPlatform {
    #[pyo3(signature = (order="breadth_first", *, cutoff=None))]
    fn sorting(&self, order: &str, cutoff: Option<i64>) -> PyResult<Self> {
        use gix::{revision::walk::Sorting, traverse::commit::simple::CommitTimeOrder};
        let sorting = match order {
            "breadth_first" if cutoff.is_none() => Sorting::BreadthFirst,
            "newest_first" | "oldest_first" => {
                let order = if order == "newest_first" {
                    CommitTimeOrder::NewestFirst
                } else {
                    CommitTimeOrder::OldestFirst
                };
                match cutoff {
                    Some(seconds) => Sorting::ByCommitTimeCutoff { order, seconds },
                    None => Sorting::ByCommitTime(order),
                }
            }
            _ => {
                return Err(PyValueError::new_err(
                    "order must be breadth_first, newest_first, or oldest_first; cutoff needs a time order",
                ));
            }
        };
        Ok(self.changed(WalkChange::Sorting(sorting)))
    }
    fn first_parent_only(&self) -> Self {
        self.changed(WalkChange::FirstParent)
    }
    fn use_commit_graph(&self, toggle: Option<bool>) -> Self {
        self.changed(WalkChange::CommitGraph(toggle))
    }
    fn with_boundary(&self, ids: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(self.changed(WalkChange::Boundary(specs(ids)?)))
    }
    fn with_hidden(&self, tips: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(self.changed(WalkChange::Hidden(specs(tips)?)))
    }
    #[pyo3(signature = (*, progress=None, cancel=None))]
    fn all(&self, progress: Option<&Progress>, cancel: Option<&CancellationToken>) -> RevisionWalk {
        let config = self.clone();
        RevisionWalk {
            inner: OwnedIter::new("revision walk", progress, cancel, move |context, producer| {
                config.handle.with(|repo| {
                    let mut walk = repo.rev_walk(resolve(&config.tips, repo)?);
                    for change in config.changes {
                        walk = match change {
                            WalkChange::Sorting(v) => walk.sorting(v),
                            WalkChange::FirstParent => walk.first_parent_only(),
                            WalkChange::CommitGraph(v) => walk.use_commit_graph(v),
                            WalkChange::Boundary(v) => walk.with_boundary(resolve(&v, repo)?),
                            WalkChange::Hidden(v) => walk.with_hidden(resolve(&v, repo)?),
                        };
                    }
                    let iter = walk
                        .selected(move |_| !context.interrupt.load(std::sync::atomic::Ordering::Acquire))
                        .map_err(to_py)?;
                    producer.serve(iter.map(|item| {
                        context.progress.inc();
                        item.map(|info| RevisionInfo {
                            handle: config.handle.clone(),
                            inner: info.detach(),
                        })
                        .map_err(to_py)
                    }))
                })
            }),
        }
    }
}

#[pyclass(frozen, module = "gix")]
pub struct RevisionWalk {
    inner: OwnedIter<RevisionInfo, PyErr>,
}
#[pymethods]
impl RevisionWalk {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<RevisionInfo>> {
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
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct RevisionInfo {
    handle: RepoHandle,
    inner: gix::traverse::commit::Info,
}
#[pymethods]
impl RevisionInfo {
    #[getter]
    fn id(&self) -> ObjectId {
        ObjectId { inner: self.inner.id }
    }
    #[getter]
    fn parent_ids(&self) -> Vec<ObjectId> {
        self.inner
            .parent_ids
            .iter()
            .map(|inner| ObjectId { inner: *inner })
            .collect()
    }
    #[getter]
    fn commit_time(&self) -> Option<i64> {
        self.inner.commit_time
    }
    #[getter]
    fn generation(&self) -> Option<u32> {
        self.inner.generation
    }
    fn object(&self, py: Python<'_>) -> PyResult<Commit> {
        let id = self.inner.id;
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            Ok(Commit {
                object: Object::from_native(handle, repo.find_commit(id).map_err(to_py)?.into()),
            })
        })
    }
}

#[pymethods]
impl Repository {
    fn rev_parse_single(&self, py: Python<'_>, spec: &Bound<'_, PyAny>) -> PyResult<ObjectId> {
        let spec = bytes(spec)?;
        self.handle.run(py, move |r| {
            r.rev_parse_single(spec.as_bstr())
                .map(|v| ObjectId { inner: v.detach() })
                .map_err(to_py)
        })
    }
    fn rev_parse(&self, py: Python<'_>, spec: &Bound<'_, PyAny>) -> PyResult<RevisionSpec> {
        let spec = bytes(spec)?;
        self.handle.run(py, move |r| {
            let spec = r.rev_parse(spec.as_bstr()).map_err(to_py)?;
            Ok(RevisionSpec {
                kind: format!("{:?}", spec.kind()),
                display: spec.to_string(),
                single: spec.single().map(|v| ObjectId { inner: v.detach() }),
                path: spec.path_and_mode().map(|(p, m)| (p.to_vec(), m.value())),
            })
        })
    }
    fn rev_walk(&self, tips: &Bound<'_, PyAny>) -> PyResult<RevisionWalkPlatform> {
        Ok(RevisionWalkPlatform::new(self.handle.clone(), specs(tips)?))
    }
    fn merge_base(&self, py: Python<'_>, one: &Bound<'_, PyAny>, two: &Bound<'_, PyAny>) -> PyResult<Option<ObjectId>> {
        let one = ObjectSpec::extract(one)?;
        let two = ObjectSpec::extract(two)?;
        self.handle.run(py, move |r| {
            r.merge_base(one.resolve(r)?, two.resolve(r)?)
                .map(|v| v.map(|v| ObjectId { inner: v.detach() }))
                .map_err(to_py)
        })
    }
    fn merge_bases_many(
        &self,
        py: Python<'_>,
        one: &Bound<'_, PyAny>,
        others: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<ObjectId>> {
        let one = ObjectSpec::extract(one)?;
        let others = specs(others)?;
        self.handle.run(py, move |r| {
            r.merge_bases_many(one.resolve(r)?, &resolve(&others, r)?)
                .map(|v| v.into_iter().map(|v| ObjectId { inner: v.detach() }).collect())
                .map_err(to_py)
        })
    }
    fn merge_base_octopus(&self, py: Python<'_>, commits: &Bound<'_, PyAny>) -> PyResult<Option<ObjectId>> {
        let commits = specs(commits)?;
        self.handle.run(py, move |r| {
            r.merge_base_octopus(resolve(&commits, r)?)
                .map(|v| v.map(|v| ObjectId { inner: v.detach() }))
                .map_err(to_py)
        })
    }
    fn is_shallow(&self, py: Python<'_>) -> PyResult<bool> {
        self.handle.run(py, |r| Ok(r.is_shallow()))
    }
    fn shallow_file(&self, py: Python<'_>) -> PyResult<PathBuf> {
        self.handle.run(py, |r| Ok(r.shallow_file()))
    }
    fn shallow_commits(&self, py: Python<'_>) -> PyResult<Option<Vec<ObjectId>>> {
        self.handle.run(py, |r| {
            r.shallow_commits()
                .map(|v| v.map(|v| v.iter().map(|inner| ObjectId { inner: *inner }).collect()))
                .map_err(to_py)
        })
    }
}

#[pymethods]
impl Commit {
    fn ancestors(&self) -> RevisionWalkPlatform {
        RevisionWalkPlatform::new(self.object.handle.clone(), vec![ObjectSpec::Id(self.object.inner.id)])
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RevisionSpec>()?;
    m.add_class::<RevisionWalkPlatform>()?;
    m.add_class::<RevisionWalk>()?;
    m.add_class::<RevisionInfo>()?;
    Ok(())
}
