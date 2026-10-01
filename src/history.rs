//! Native commit descriptions, graph caches, mailmaps, and signature operations.

#[cfg(any(feature = "revision", feature = "signing"))]
use crate::objects::Object;
use crate::{
    error::to_py,
    objects::Commit,
    repository::{RepoHandle, Repository},
    runtime::OwnedIter,
    types::{HashKind, ObjectId, ObjectSpec},
};
use pyo3::{
    exceptions::{PyIndexError, PyRuntimeError},
    prelude::*,
    types::PyBytes,
};
use std::sync::Arc;

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct CommitGraph {
    handle: RepoHandle,
    inner: Arc<gix::commitgraph::Graph>,
}

#[pymethods]
impl Repository {
    fn commit_graph(&self, py: Python<'_>) -> PyResult<CommitGraph> {
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            repo.commit_graph()
                .map(|inner| CommitGraph {
                    handle,
                    inner: Arc::new(inner),
                })
                .map_err(to_py)
        })
    }
    fn commit_graph_if_enabled(&self, py: Python<'_>) -> PyResult<Option<CommitGraph>> {
        let handle = self.handle.clone();
        self.handle.run(py, move |repo| {
            repo.commit_graph_if_enabled()
                .map(|value| {
                    value.map(|inner| CommitGraph {
                        handle,
                        inner: Arc::new(inner),
                    })
                })
                .map_err(to_py)
        })
    }
}

impl CommitGraph {
    fn position(&self, position: u32) -> PyResult<gix::commitgraph::Position> {
        if position >= self.inner.num_commits() {
            return Err(PyIndexError::new_err("commit-graph position out of range"));
        }
        Ok(gix::commitgraph::Position(position))
    }
}
#[pymethods]
impl CommitGraph {
    fn num_commits(&self) -> u32 {
        self.inner.num_commits()
    }
    fn object_hash(&self) -> HashKind {
        HashKind {
            inner: self.inner.object_hash(),
        }
    }
    fn id_at(&self, position: u32) -> PyResult<ObjectId> {
        Ok(ObjectId {
            inner: self.inner.id_at(self.position(position)?).to_owned(),
        })
    }
    fn commit_at(&self, position: u32) -> PyResult<CommitGraphCommit> {
        self.position(position)?;
        Ok(CommitGraphCommit {
            graph: self.inner.clone(),
            position,
        })
    }
    fn lookup(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Option<u32>> {
        let id = ObjectSpec::extract(id)?;
        let graph = self.inner.clone();
        self.handle
            .run(py, move |repo| Ok(graph.lookup(id.resolve(repo)?).map(|p| p.0)))
    }
    fn commit_by_id(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<Option<CommitGraphCommit>> {
        Ok(self.lookup(py, id)?.map(|position| CommitGraphCommit {
            graph: self.inner.clone(),
            position,
        }))
    }
    fn iter_ids(&self) -> CommitGraphIds {
        let graph = self.inner.clone();
        CommitGraphIds {
            inner: OwnedIter::new("commit graph IDs", None, None, move |_, producer| {
                producer.serve(graph.iter_ids().map(|id| Ok(ObjectId { inner: id.to_owned() })))
            }),
        }
    }
    fn iter_commits(&self) -> CommitGraphCommits {
        let graph = self.inner.clone();
        CommitGraphCommits {
            inner: OwnedIter::new("commit graph commits", None, None, move |_, producer| {
                producer.serve((0..graph.num_commits()).map(|position| {
                    Ok(CommitGraphCommit {
                        graph: graph.clone(),
                        position,
                    })
                }))
            }),
        }
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct CommitGraphCommit {
    graph: Arc<gix::commitgraph::Graph>,
    position: u32,
}
#[pymethods]
impl CommitGraphCommit {
    fn id(&self) -> ObjectId {
        ObjectId {
            inner: self.graph.id_at(gix::commitgraph::Position(self.position)).to_owned(),
        }
    }
    fn committer_timestamp(&self) -> u64 {
        self.graph
            .commit_at(gix::commitgraph::Position(self.position))
            .committer_timestamp()
    }
    fn generation(&self) -> u32 {
        self.graph
            .commit_at(gix::commitgraph::Position(self.position))
            .generation()
    }
    fn root_tree_id(&self) -> ObjectId {
        ObjectId {
            inner: self
                .graph
                .commit_at(gix::commitgraph::Position(self.position))
                .root_tree_id()
                .to_owned(),
        }
    }
    fn parent1(&self) -> PyResult<Option<u32>> {
        self.graph
            .commit_at(gix::commitgraph::Position(self.position))
            .parent1()
            .map(|p| p.map(|p| p.0))
            .map_err(to_py)
    }
    fn iter_parents(&self) -> CommitGraphParents {
        let graph = self.graph.clone();
        let position = self.position;
        CommitGraphParents {
            inner: OwnedIter::new("commit graph parents", None, None, move |_, producer| {
                producer.serve(
                    graph
                        .commit_at(gix::commitgraph::Position(position))
                        .iter_parents()
                        .map(|p| p.map(|p| p.0).map_err(to_py)),
                )
            }),
        }
    }
}

macro_rules! iterator {
    ($name:ident, $item:ty) => {
        #[pyclass(frozen, module = "gix")]
        pub struct $name {
            inner: OwnedIter<$item, PyErr>,
        }
        #[pymethods]
        impl $name {
            fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                slf
            }
            fn __next__(&self, py: Python<'_>) -> PyResult<Option<$item>> {
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
    };
}
iterator!(CommitGraphIds, ObjectId);
iterator!(CommitGraphCommits, CommitGraphCommit);
iterator!(CommitGraphParents, u32);

#[cfg(feature = "revision")]
mod revision {
    use super::*;
    use crate::runtime::CommandOwner;

    enum Command {
        Len,
        Clear,
        Contains(ObjectSpec),
        Base(gix::ObjectId, gix::ObjectId),
        Many(gix::ObjectId, Vec<gix::ObjectId>),
        Octopus(Vec<gix::ObjectId>),
    }
    enum Reply {
        Len(usize),
        Flag(bool),
        Ids(Vec<ObjectId>),
    }
    #[pyclass(frozen, module = "gix")]
    pub struct RevisionGraph {
        owner: CommandOwner<Command, Reply>,
    }
    impl RevisionGraph {
        fn ids(&self, py: Python<'_>, command: Command) -> PyResult<Vec<ObjectId>> {
            match self.owner.call(py, command)? {
                Reply::Ids(ids) => Ok(ids),
                _ => Err(PyRuntimeError::new_err("unexpected graph response")),
            }
        }
    }
    #[pymethods]
    impl RevisionGraph {
        fn len(&self, py: Python<'_>) -> PyResult<usize> {
            match self.owner.call(py, Command::Len)? {
                Reply::Len(n) => Ok(n),
                _ => Err(PyRuntimeError::new_err("unexpected graph response")),
            }
        }
        fn is_empty(&self, py: Python<'_>) -> PyResult<bool> {
            Ok(self.len(py)? == 0)
        }
        fn contains(&self, py: Python<'_>, id: &Bound<'_, PyAny>) -> PyResult<bool> {
            match self.owner.call(py, Command::Contains(ObjectSpec::extract(id)?))? {
                Reply::Flag(value) => Ok(value),
                _ => Err(PyRuntimeError::new_err("unexpected graph response")),
            }
        }
        fn clear(&self, py: Python<'_>) -> PyResult<()> {
            self.owner.call(py, Command::Clear)?;
            Ok(())
        }
        fn close(&self, py: Python<'_>) -> PyResult<()> {
            self.owner.close(py)
        }
    }
    #[pymethods]
    impl Repository {
        #[pyo3(signature = (cache=None))]
        fn revision_graph(&self, cache: Option<CommitGraph>) -> RevisionGraph {
            let handle = self.handle.clone();
            RevisionGraph {
                owner: CommandOwner::new("revision graph", move |_, commands, producer| {
                    handle.with(|repo| {
                        let mut graph = repo.revision_graph(cache.as_ref().map(|g| g.inner.as_ref()));
                        producer.serve(std::iter::from_fn(|| {
                            let command = commands.lock().unwrap_or_else(|e| e.into_inner()).take()?;
                            Some(Ok((|| -> PyResult<Reply> {
                                Ok(match command {
                                    Command::Len => Reply::Len(graph.len()),
                                    Command::Clear => {
                                        graph.clear();
                                        Reply::Len(0)
                                    }
                                    Command::Contains(id) => Reply::Flag(graph.contains(&id.resolve(repo)?)),
                                    Command::Base(one, two) => Reply::Ids(
                                        repo.merge_base_with_graph(one, two, &mut graph)
                                            .map_err(to_py)?
                                            .into_iter()
                                            .map(|id| ObjectId { inner: id.detach() })
                                            .collect(),
                                    ),
                                    Command::Many(one, others) => Reply::Ids(
                                        repo.merge_bases_many_with_graph(one, &others, &mut graph)
                                            .map_err(to_py)?
                                            .into_iter()
                                            .map(|id| ObjectId { inner: id.detach() })
                                            .collect(),
                                    ),
                                    Command::Octopus(ids) => Reply::Ids(
                                        repo.merge_base_octopus_with_graph(ids, &mut graph)
                                            .map_err(to_py)?
                                            .into_iter()
                                            .map(|id| ObjectId { inner: id.detach() })
                                            .collect(),
                                    ),
                                })
                            })()))
                        }))
                    })
                }),
            }
        }
        fn merge_base_with_graph(
            &self,
            py: Python<'_>,
            one: &Bound<'_, PyAny>,
            two: &Bound<'_, PyAny>,
            graph: &RevisionGraph,
        ) -> PyResult<Option<ObjectId>> {
            let one = ObjectSpec::extract(one)?;
            let two = ObjectSpec::extract(two)?;
            let (one, two) = self
                .handle
                .run(py, move |repo| Ok((one.resolve(repo)?, two.resolve(repo)?)))?;
            Ok(graph.ids(py, Command::Base(one, two))?.into_iter().next())
        }
        fn merge_bases_many_with_graph(
            &self,
            py: Python<'_>,
            one: &Bound<'_, PyAny>,
            others: &Bound<'_, PyAny>,
            graph: &RevisionGraph,
        ) -> PyResult<Vec<ObjectId>> {
            let one = ObjectSpec::extract(one)?;
            let others = others
                .try_iter()?
                .map(|v| ObjectSpec::extract(&v?))
                .collect::<PyResult<Vec<_>>>()?;
            let (one, others) = self.handle.run(py, move |repo| {
                Ok((
                    one.resolve(repo)?,
                    others.iter().map(|v| v.resolve(repo)).collect::<PyResult<Vec<_>>>()?,
                ))
            })?;
            graph.ids(py, Command::Many(one, others))
        }
        fn merge_base_octopus_with_graph(
            &self,
            py: Python<'_>,
            commits: &Bound<'_, PyAny>,
            graph: &RevisionGraph,
        ) -> PyResult<Option<ObjectId>> {
            let commits = commits
                .try_iter()?
                .map(|v| ObjectSpec::extract(&v?))
                .collect::<PyResult<Vec<_>>>()?;
            let commits = self.handle.run(py, move |repo| {
                commits.iter().map(|v| v.resolve(repo)).collect::<PyResult<Vec<_>>>()
            })?;
            Ok(graph.ids(py, Command::Octopus(commits))?.into_iter().next())
        }
    }

    #[pyclass(frozen, module = "gix", from_py_object, eq)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct SelectRef {
        inner: gix::commit::describe::SelectRef,
    }
    #[pymethods]
    impl SelectRef {
        #[classattr]
        #[pyo3(name = "AnnotatedTags")]
        fn annotated() -> Self {
            Self {
                inner: gix::commit::describe::SelectRef::AnnotatedTags,
            }
        }
        #[classattr]
        #[pyo3(name = "AllTags")]
        fn tags() -> Self {
            Self {
                inner: gix::commit::describe::SelectRef::AllTags,
            }
        }
        #[classattr]
        #[pyo3(name = "AllRefs")]
        fn refs() -> Self {
            Self {
                inner: gix::commit::describe::SelectRef::AllRefs,
            }
        }
    }
    #[pyclass(module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct DescribePlatform {
        object: Object,
        select: SelectRef,
        first_parent: bool,
        fallback: bool,
        max_candidates: usize,
    }
    #[pymethods]
    impl Commit {
        fn describe(&self) -> DescribePlatform {
            DescribePlatform {
                object: self.object.clone(),
                select: SelectRef::annotated(),
                first_parent: false,
                fallback: false,
                max_candidates: 10,
            }
        }
    }
    impl DescribePlatform {
        fn resolve(
            &self,
            py: Python<'_>,
            cache: Option<CommitGraph>,
            use_cache: bool,
        ) -> PyResult<Option<DescribeResolution>> {
            let config = self.clone();
            let handle = self.object.handle.clone();
            self.object.handle.run(py, move |repo| {
                let commit = config
                    .object
                    .inner
                    .as_ref()
                    .clone()
                    .attach(repo)
                    .try_into_commit()
                    .map_err(to_py)?;
                let platform = commit
                    .describe()
                    .names(config.select.inner)
                    .traverse_first_parent(config.first_parent)
                    .id_as_fallback(config.fallback)
                    .max_candidates(config.max_candidates);
                let result = if use_cache {
                    platform.try_resolve_with_cache(cache.as_ref().map(|c| c.inner.as_ref()))
                } else {
                    platform.try_resolve()
                }
                .map_err(to_py)?;
                Ok(result.map(|resolution| DescribeResolution {
                    handle,
                    inner: resolution.outcome,
                }))
            })
        }
    }
    #[pymethods]
    impl DescribePlatform {
        fn names(&self, select: SelectRef) -> Self {
            Self { select, ..self.clone() }
        }
        fn traverse_first_parent(&self, first_parent: bool) -> Self {
            Self {
                first_parent,
                ..self.clone()
            }
        }
        fn max_candidates(&self, candidates: usize) -> Self {
            Self {
                max_candidates: candidates,
                ..self.clone()
            }
        }
        fn id_as_fallback(&self, use_fallback: bool) -> Self {
            Self {
                fallback: use_fallback,
                ..self.clone()
            }
        }
        fn try_resolve(&self, py: Python<'_>) -> PyResult<Option<DescribeResolution>> {
            self.resolve(py, None, false)
        }
        #[pyo3(signature = (cache=None))]
        fn try_resolve_with_cache(
            &self,
            py: Python<'_>,
            cache: Option<CommitGraph>,
        ) -> PyResult<Option<DescribeResolution>> {
            self.resolve(py, cache, true)
        }
        fn try_format(&self, py: Python<'_>) -> PyResult<Option<DescribeFormat>> {
            self.try_resolve(py)?.map(|r| r.format(py)).transpose()
        }
        fn format(&mut self, py: Python<'_>) -> PyResult<DescribeFormat> {
            self.fallback = true;
            self.try_format(py)?
                .ok_or_else(|| PyRuntimeError::new_err("describe fallback produced no result"))
        }
    }
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct DescribeResolution {
        handle: RepoHandle,
        inner: gix::revision::plumbing::describe::Outcome<'static>,
    }
    #[pymethods]
    impl DescribeResolution {
        #[getter]
        fn id(&self) -> ObjectId {
            ObjectId { inner: self.inner.id }
        }
        #[getter]
        fn name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.name.as_deref().map(|n| PyBytes::new(py, n))
        }
        #[getter]
        fn depth(&self) -> u32 {
            self.inner.depth
        }
        #[getter]
        fn commits_seen(&self) -> u32 {
            self.inner.commits_seen
        }
        fn format(&self, py: Python<'_>) -> PyResult<DescribeFormat> {
            let inner = self.inner.clone();
            self.handle.run(py, move |repo| {
                use gix::prelude::ObjectIdExt;
                gix::commit::describe::Resolution {
                    id: inner.id.attach(repo),
                    outcome: inner,
                }
                .format()
                .map(|inner| DescribeFormat { inner })
                .map_err(to_py)
            })
        }
        #[cfg(feature = "status")]
        #[pyo3(signature = (dirty_suffix=None))]
        fn format_with_dirty_suffix(&self, py: Python<'_>, dirty_suffix: Option<String>) -> PyResult<DescribeFormat> {
            let inner = self.inner.clone();
            self.handle.run(py, move |repo| {
                use gix::prelude::ObjectIdExt;
                gix::commit::describe::Resolution {
                    id: inner.id.attach(repo),
                    outcome: inner,
                }
                .format_with_dirty_suffix(dirty_suffix)
                .map(|inner| DescribeFormat { inner })
                .map_err(to_py)
            })
        }
    }
    #[pyclass(module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct DescribeFormat {
        inner: gix::revision::plumbing::describe::Format<'static>,
    }
    #[pymethods]
    impl DescribeFormat {
        #[getter]
        fn id(&self) -> ObjectId {
            ObjectId { inner: self.inner.id }
        }
        #[getter]
        fn name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.name.as_deref().map(|n| PyBytes::new(py, n))
        }
        #[getter]
        fn depth(&self) -> u32 {
            self.inner.depth
        }
        #[getter]
        fn hex_len(&self) -> usize {
            self.inner.hex_len
        }
        fn is_exact_match(&self) -> bool {
            self.inner.is_exact_match()
        }
        fn long(mut slf: PyRefMut<'_, Self>, long: bool) -> PyRefMut<'_, Self> {
            slf.inner.long(long);
            slf
        }
        fn __str__(&self) -> String {
            self.inner.to_string()
        }
    }
    pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<RevisionGraph>()?;
        m.add_class::<SelectRef>()?;
        m.add_class::<DescribePlatform>()?;
        m.add_class::<DescribeResolution>()?;
        m.add_class::<DescribeFormat>()?;
        Ok(())
    }
}

#[cfg(feature = "mailmap")]
mod mailmap {
    use super::*;
    use crate::objects::Signature;
    use std::sync::RwLock;
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct Mailmap {
        inner: Arc<RwLock<Arc<gix::mailmap::Snapshot>>>,
    }
    impl Mailmap {
        fn snapshot(&self) -> PyResult<Arc<gix::mailmap::Snapshot>> {
            Ok(self.inner.read().map_err(to_py)?.clone())
        }
    }
    #[pymethods]
    impl Repository {
        fn open_mailmap(&self, py: Python<'_>) -> PyResult<Mailmap> {
            self.handle.run(py, |repo| {
                Ok(Mailmap {
                    inner: Arc::new(RwLock::new(Arc::new(repo.open_mailmap()))),
                })
            })
        }
        fn open_mailmap_into(&self, py: Python<'_>, target: Mailmap) -> PyResult<()> {
            self.handle.run(py, move |repo| {
                let mut target = target.inner.write().map_err(to_py)?;
                repo.open_mailmap_into(Arc::make_mut(&mut *target)).map_err(to_py)
            })
        }
    }
    #[pymethods]
    impl Mailmap {
        fn try_resolve(&self, py: Python<'_>, signature: Signature) -> PyResult<Option<Signature>> {
            let snapshot = self.snapshot()?;
            Ok(py.detach(move || {
                snapshot
                    .try_resolve(signature.inner.to_ref(&mut gix::date::parse::TimeBuf::default()))
                    .map(|inner| Signature { inner })
            }))
        }
        fn resolve(&self, py: Python<'_>, signature: Signature) -> PyResult<Signature> {
            let snapshot = self.snapshot()?;
            Ok(py.detach(move || Signature {
                inner: snapshot.resolve(signature.inner.to_ref(&mut gix::date::parse::TimeBuf::default())),
            }))
        }
        fn iter(&self) -> PyResult<MailmapEntries> {
            let snapshot = self.snapshot()?;
            Ok(MailmapEntries {
                inner: OwnedIter::new("mailmap entries", None, None, move |_, producer| {
                    producer.serve(snapshot.iter().map(|entry| Ok(MailmapEntry::from_native(entry))))
                }),
            })
        }
        fn entries(&self, py: Python<'_>) -> PyResult<Vec<MailmapEntry>> {
            let snapshot = self.snapshot()?;
            Ok(py.detach(move || snapshot.entries().into_iter().map(MailmapEntry::from_native).collect()))
        }
    }
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct MailmapEntry {
        new_name: Option<Vec<u8>>,
        new_email: Option<Vec<u8>>,
        old_name: Option<Vec<u8>>,
        old_email: Vec<u8>,
    }
    impl MailmapEntry {
        fn from_native(entry: gix::mailmap::Entry<'_>) -> Self {
            Self {
                new_name: entry.new_name().map(|v| v.to_vec()),
                new_email: entry.new_email().map(|v| v.to_vec()),
                old_name: entry.old_name().map(|v| v.to_vec()),
                old_email: entry.old_email().to_vec(),
            }
        }
    }
    #[pymethods]
    impl MailmapEntry {
        #[getter]
        fn new_name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.new_name.as_deref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn new_email<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.new_email.as_deref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn old_name<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.old_name.as_deref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn old_email<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
            PyBytes::new(py, &self.old_email)
        }
    }
    iterator!(MailmapEntries, MailmapEntry);
    pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<Mailmap>()?;
        m.add_class::<MailmapEntry>()?;
        m.add_class::<MailmapEntries>()?;
        Ok(())
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct SignedData {
    object: Arc<gix::ObjectDetached>,
}
#[pymethods]
impl SignedData {
    fn to_bstring<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let value = py.detach(|| {
            Ok::<_, PyErr>(
                gix::objs::CommitRefIter::signature(&self.object.data, self.object.id.kind())
                    .map_err(to_py)?
                    .ok_or_else(|| PyRuntimeError::new_err("signature is absent"))?
                    .1
                    .to_bstring(),
            )
        })?;
        Ok(PyBytes::new(py, &value))
    }
}
#[pymethods]
impl Commit {
    fn signature<'py>(&self, py: Python<'py>) -> PyResult<Option<(Bound<'py, PyBytes>, SignedData)>> {
        let value = py.detach(|| {
            gix::objs::CommitRefIter::signature(&self.object.inner.data, self.object.inner.id.kind())
                .map(|value| value.map(|(signature, _)| signature.into_owned()))
                .map_err(to_py)
        })?;
        Ok(value.map(|signature| {
            (
                PyBytes::new(py, &signature),
                SignedData {
                    object: self.object.inner.clone(),
                },
            )
        }))
    }
}

#[cfg(feature = "signing")]
mod signing {
    use super::*;
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct SigningOptions {
        inner: gix::commit::sign::Options,
    }
    #[pymethods]
    impl SigningOptions {
        #[getter]
        fn format(&self) -> String {
            format!("{:?}", self.inner.format)
        }
        #[getter]
        fn program<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
            PyBytes::new(py, &gix::path::into_bstr(std::path::PathBuf::from(&self.inner.program)))
        }
        #[getter]
        fn signing_key<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
            PyBytes::new(
                py,
                &gix::path::into_bstr(std::path::PathBuf::from(&self.inner.signing_key)),
            )
        }
    }
    #[pymethods]
    impl Repository {
        fn commit_signing_options(&self, py: Python<'_>) -> PyResult<SigningOptions> {
            self.handle.run(py, |repo| {
                repo.commit_signing_options()
                    .map(|inner| SigningOptions { inner })
                    .map_err(to_py)
            })
        }
        fn commit_signing_options_if_enabled(&self, py: Python<'_>) -> PyResult<Option<SigningOptions>> {
            self.handle.run(py, |repo| {
                repo.commit_signing_options_if_enabled()
                    .map(|v| v.map(|inner| SigningOptions { inner }))
                    .map_err(to_py)
            })
        }
    }
    #[pymethods]
    impl Commit {
        fn signed(&self, py: Python<'_>) -> PyResult<Commit> {
            let object = self.object.inner.clone();
            let handle = self.object.handle.clone();
            self.object.handle.run(py, move |repo| {
                let commit = object.as_ref().clone().attach(repo).try_into_commit().map_err(to_py)?;
                Ok(Commit {
                    object: Object::from_native(handle, commit.signed().map_err(to_py)?.into()),
                })
            })
        }
        fn verify_signature(&self, py: Python<'_>) -> PyResult<Option<VerificationOutcome>> {
            let object = self.object.inner.clone();
            self.object.handle.run(py, move |repo| {
                object
                    .as_ref()
                    .clone()
                    .attach(repo)
                    .try_into_commit()
                    .map_err(to_py)?
                    .verify_signature()
                    .map(|v| v.map(|inner| VerificationOutcome { inner }))
                    .map_err(to_py)
            })
        }
    }
    #[pyclass(frozen, module = "gix", from_py_object)]
    #[derive(Clone)]
    pub struct VerificationOutcome {
        inner: gix::commit::verify::Outcome,
    }
    #[pymethods]
    impl VerificationOutcome {
        fn is_valid(&self) -> bool {
            self.inner.is_valid()
        }
        #[getter]
        fn format(&self) -> String {
            format!("{:?}", self.inner.format)
        }
        #[getter]
        fn status(&self) -> String {
            format!("{:?}", self.inner.status)
        }
        #[getter]
        fn trust_level(&self) -> String {
            format!("{:?}", self.inner.trust_level)
        }
        #[getter]
        fn signer<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.signer.as_ref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn key<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.key.as_ref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn fingerprint<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.fingerprint.as_ref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn primary_key_fingerprint<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.primary_key_fingerprint.as_ref().map(|v| PyBytes::new(py, v))
        }
        #[getter]
        fn output<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
            PyBytes::new(py, &self.inner.output)
        }
        #[getter]
        fn raw_output<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
            PyBytes::new(py, &self.inner.raw_output)
        }
    }
    pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<SigningOptions>()?;
        m.add_class::<VerificationOutcome>()?;
        Ok(())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<CommitGraph>()?;
    m.add_class::<CommitGraphCommit>()?;
    m.add_class::<CommitGraphIds>()?;
    m.add_class::<CommitGraphCommits>()?;
    m.add_class::<CommitGraphParents>()?;
    m.add_class::<SignedData>()?;
    #[cfg(feature = "revision")]
    revision::register(m)?;
    #[cfg(feature = "mailmap")]
    mailmap::register(m)?;
    #[cfg(feature = "signing")]
    signing::register(m)?;
    Ok(())
}
