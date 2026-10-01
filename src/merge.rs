use crate::{
    diff::TreeChange,
    diff_options::Rewrites,
    error::to_py,
    index::IndexFile,
    objects::TreeEditor,
    repository::Repository,
    runtime::{self, CancellationToken, Progress},
    types::{ObjectId, ObjectSpec, bytes},
};
use gix::bstr::ByteSlice;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::sync::{Arc, Mutex};

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Copy, Default)]
pub struct TreatAsUnresolved {
    inner: gix::merge::tree::TreatAsUnresolved,
}
#[pymethods]
impl TreatAsUnresolved {
    #[staticmethod]
    fn git() -> Self {
        Self {
            inner: gix::merge::tree::TreatAsUnresolved::git(),
        }
    }
    #[staticmethod]
    fn forced_resolution() -> Self {
        Self {
            inner: gix::merge::tree::TreatAsUnresolved::forced_resolution(),
        }
    }
    #[staticmethod]
    fn undecidable() -> Self {
        Self {
            inner: gix::merge::tree::TreatAsUnresolved::undecidable(),
        }
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct TreeMergeOptions {
    inner: gix::merge::tree::Options,
}
#[pymethods]
impl TreeMergeOptions {
    fn with_rewrites(&self, rewrites: Option<Rewrites>) -> Self {
        Self {
            inner: self.inner.clone().with_rewrites(rewrites.map(|v| v.inner)),
        }
    }
    fn with_fail_on_conflict(&self, how: Option<TreatAsUnresolved>) -> Self {
        Self {
            inner: self.inner.clone().with_fail_on_conflict(how.map(|v| v.inner)),
        }
    }
    fn with_file_favor(&self, favor: Option<&str>) -> PyResult<Self> {
        use gix::merge::tree::FileFavor;
        let favor = match favor {
            None => None,
            Some("ours") => Some(FileFavor::Ours),
            Some("theirs") => Some(FileFavor::Theirs),
            _ => return Err(PyValueError::new_err("file favor must be ours, theirs, or None")),
        };
        Ok(Self {
            inner: self.inner.clone().with_file_favor(favor),
        })
    }
    fn with_tree_favor(&self, favor: Option<&str>) -> PyResult<Self> {
        use gix::merge::tree::TreeFavor;
        let favor = match favor {
            None => None,
            Some("ours") => Some(TreeFavor::Ours),
            Some("ancestor") => Some(TreeFavor::Ancestor),
            _ => return Err(PyValueError::new_err("tree favor must be ours, ancestor, or None")),
        };
        Ok(Self {
            inner: self.inner.clone().with_tree_favor(favor),
        })
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct CommitMergeOptions {
    inner: gix::merge::commit::Options,
}
#[pymethods]
impl CommitMergeOptions {
    #[new]
    fn new(tree_merge: TreeMergeOptions) -> Self {
        Self {
            inner: tree_merge.inner.into(),
        }
    }
    fn with_allow_missing_merge_base(&self, allow: bool) -> Self {
        Self {
            inner: self.inner.clone().with_allow_missing_merge_base(allow),
        }
    }
    fn with_use_first_merge_base(&self, use_first: bool) -> Self {
        Self {
            inner: self.inner.clone().with_use_first_merge_base(use_first),
        }
    }
}
#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone, Default)]
pub struct MergeLabels {
    ancestor: Option<Vec<u8>>,
    current: Option<Vec<u8>>,
    other: Option<Vec<u8>>,
}
impl MergeLabels {
    pub(crate) fn native(&self) -> gix::merge::blob::builtin_driver::text::Labels<'_> {
        gix::merge::blob::builtin_driver::text::Labels {
            ancestor: self.ancestor.as_deref().map(ByteSlice::as_bstr),
            current: self.current.as_deref().map(ByteSlice::as_bstr),
            other: self.other.as_deref().map(ByteSlice::as_bstr),
        }
    }
}
#[pymethods]
impl MergeLabels {
    #[new]
    #[pyo3(signature=(*,ancestor=None,current=None,other=None))]
    fn new(
        ancestor: Option<&Bound<'_, PyAny>>,
        current: Option<&Bound<'_, PyAny>>,
        other: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        Ok(Self {
            ancestor: ancestor.map(bytes).transpose()?,
            current: current.map(bytes).transpose()?,
            other: other.map(bytes).transpose()?,
        })
    }
}

#[pyclass(frozen, module = "gix", from_py_object)]
#[derive(Clone)]
pub struct MergeConflict {
    inner: gix::merge::tree::Conflict,
}
#[pymethods]
impl MergeConflict {
    #[getter]
    fn ours(&self) -> TreeChange {
        TreeChange {
            inner: self.inner.ours.clone(),
        }
    }
    #[getter]
    fn theirs(&self) -> TreeChange {
        TreeChange {
            inner: self.inner.theirs.clone(),
        }
    }
    fn changes_in_resolution(&self) -> (TreeChange, TreeChange) {
        let (a, b) = self.inner.changes_in_resolution();
        (TreeChange { inner: a.clone() }, TreeChange { inner: b.clone() })
    }
    fn is_unresolved(&self, how: TreatAsUnresolved) -> bool {
        self.inner.is_unresolved(how.inner)
    }
    fn entries(&self) -> Vec<Option<MergeIndexEntry>> {
        self.inner
            .entries()
            .into_iter()
            .map(|v| v.map(|inner| MergeIndexEntry { inner }))
            .collect()
    }
    fn content_merge(&self) -> Option<ContentMerge> {
        self.inner.content_merge().map(|inner| ContentMerge { inner })
    }
    #[getter]
    fn resolution<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        use gix::merge::tree::Resolution;
        let out = PyDict::new(py);
        out.set_item("ok", self.inner.resolution.is_ok())?;
        match &self.inner.resolution {
            Ok(Resolution::SourceLocationAffectedByRename { final_location }) => {
                out.set_item("kind", "SourceLocationAffectedByRename")?;
                out.set_item("final_location", PyBytes::new(py, final_location))?;
            }
            Ok(Resolution::OursModifiedTheirsRenamedAndChangedThenRename {
                merged_mode,
                merged_blob,
                final_location,
            }) => {
                out.set_item("kind", "OursModifiedTheirsRenamedAndChangedThenRename")?;
                out.set_item("merged_mode", merged_mode.map(|v| v.value()))?;
                out.set_item("merged_blob", merged_blob.map(|inner| ContentMerge { inner }))?;
                out.set_item("final_location", final_location.as_ref().map(|v| PyBytes::new(py, v)))?;
            }
            Ok(Resolution::OursModifiedTheirsModifiedThenBlobContentMerge { merged_blob }) => {
                out.set_item("kind", "OursModifiedTheirsModifiedThenBlobContentMerge")?;
                out.set_item("merged_blob", ContentMerge { inner: *merged_blob })?;
            }
            Ok(Resolution::Forced(failure)) => {
                out.set_item("kind", "Forced")?;
                let fields = PyDict::new(py);
                failure_fields(py, failure, &fields)?;
                out.set_item("failure", fields)?;
            }
            Err(failure) => failure_fields(py, failure, &out)?,
        }
        Ok(out)
    }
}
fn failure_fields(
    py: Python<'_>,
    failure: &gix::merge::tree::ResolutionFailure,
    out: &Bound<'_, PyDict>,
) -> PyResult<()> {
    use gix::merge::tree::ResolutionFailure::*;
    let kind = match failure {
        SubmoduleMerge => "SubmoduleMerge",
        SubmoduleAddAdd => "SubmoduleAddAdd",
        OursRenamedTheirsRenamedToSameLocation => "OursRenamedTheirsRenamedToSameLocation",
        OursRenamedTheirsRenamedDifferently { merged_blob } => {
            out.set_item("merged_blob", merged_blob.map(|inner| ContentMerge { inner }))?;
            "OursRenamedTheirsRenamedDifferently"
        }
        OursModifiedTheirsDirectoryThenOursRenamed {
            renamed_unique_path_to_modified_blob,
        } => {
            out.set_item(
                "renamed_unique_path_to_modified_blob",
                PyBytes::new(py, renamed_unique_path_to_modified_blob),
            )?;
            "OursModifiedTheirsDirectoryThenOursRenamed"
        }
        OursDirectoryTheirsNonDirectoryTheirsRenamed {
            renamed_unique_path_of_theirs,
        } => {
            out.set_item(
                "renamed_unique_path_of_theirs",
                PyBytes::new(py, renamed_unique_path_of_theirs),
            )?;
            "OursDirectoryTheirsNonDirectoryTheirsRenamed"
        }
        OursAddedTheirsAddedTypeMismatch { their_unique_location } => {
            out.set_item("their_unique_location", PyBytes::new(py, their_unique_location))?;
            "OursAddedTheirsAddedTypeMismatch"
        }
        OursModifiedTheirsRenamedTypeMismatch => "OursModifiedTheirsRenamedTypeMismatch",
        OursDeletedTheirsRenamed => "OursDeletedTheirsRenamed",
        OursModifiedTheirsDeleted => "OursModifiedTheirsDeleted",
        Unknown => "Unknown",
    };
    out.set_item("kind", kind)
}
#[pyclass(frozen, module = "gix")]
pub struct MergeIndexEntry {
    inner: gix::merge::plumbing::tree::ConflictIndexEntry,
}
#[pymethods]
impl MergeIndexEntry {
    #[getter]
    fn id(&self) -> ObjectId {
        ObjectId { inner: self.inner.id }
    }
    #[getter]
    fn mode(&self) -> u16 {
        self.inner.mode.value()
    }
}
#[pyclass(frozen, module = "gix")]
pub struct ContentMerge {
    inner: gix::merge::tree::ContentMerge,
}
#[pymethods]
impl ContentMerge {
    #[getter]
    fn merged_blob_id(&self) -> ObjectId {
        ObjectId {
            inner: self.inner.merged_blob_id,
        }
    }
    #[getter]
    fn resolution(&self) -> String {
        format!("{:?}", self.inner.resolution)
    }
}

#[pyclass(frozen, module = "gix")]
pub struct TreeMergeOutcome {
    tree: Py<TreeEditor>,
    conflicts: Arc<Vec<gix::merge::tree::Conflict>>,
    failed: bool,
}
#[pymethods]
impl TreeMergeOutcome {
    #[getter]
    fn tree(&self, py: Python<'_>) -> Py<TreeEditor> {
        self.tree.clone_ref(py)
    }
    #[getter]
    fn conflicts(&self) -> Vec<MergeConflict> {
        self.conflicts
            .iter()
            .cloned()
            .map(|inner| MergeConflict { inner })
            .collect()
    }
    #[getter]
    fn failed_on_first_unresolved_conflict(&self) -> bool {
        self.failed
    }
    fn has_unresolved_conflicts(&self, how: TreatAsUnresolved) -> bool {
        self.conflicts.iter().any(|c| c.is_unresolved(how.inner))
    }
    #[pyo3(signature=(index,how,removal_mode="prune"))]
    fn index_changed_after_applying_conflicts(
        &self,
        py: Python<'_>,
        index: &IndexFile,
        how: TreatAsUnresolved,
        removal_mode: &str,
    ) -> PyResult<bool> {
        use gix::merge::tree::apply_index_entries::RemovalMode;
        let mode = match removal_mode {
            "prune" => RemovalMode::Prune,
            "mark" => RemovalMode::Mark,
            _ => return Err(PyValueError::new_err("removal_mode must be prune or mark")),
        };
        let conflicts = self.conflicts.clone();
        index.mutate(py, move |index| {
            Ok(gix::merge::tree::apply_index_entries(
                &conflicts, how.inner, index, mode,
            ))
        })
    }
}
#[pyclass(frozen, module = "gix")]
pub struct CommitMergeOutcome {
    #[pyo3(get)]
    tree_merge: Py<TreeMergeOutcome>,
    #[pyo3(get)]
    merge_base_tree_id: ObjectId,
    #[pyo3(get)]
    merge_bases: Option<Vec<ObjectId>>,
    #[pyo3(get)]
    virtual_merge_bases: Vec<ObjectId>,
}
#[pyclass(frozen, module = "gix")]
pub struct VirtualMergeBase {
    #[pyo3(get)]
    commit_id: ObjectId,
    #[pyo3(get)]
    tree_id: ObjectId,
    #[pyo3(get)]
    virtual_merge_bases: Vec<ObjectId>,
}

#[pymethods]
impl Repository {
    fn tree_merge_options(&self, py: Python<'_>) -> PyResult<TreeMergeOptions> {
        self.handle.run(py, |r| {
            r.tree_merge_options()
                .map(|inner| TreeMergeOptions { inner })
                .map_err(to_py)
        })
    }
    #[pyo3(signature=(ancestor_tree,our_tree,their_tree,labels=None,options=None,*,progress=None,cancel=None))]
    #[allow(clippy::too_many_arguments)]
    fn merge_trees(
        &self,
        py: Python<'_>,
        ancestor_tree: &Bound<'_, PyAny>,
        our_tree: &Bound<'_, PyAny>,
        their_tree: &Bound<'_, PyAny>,
        labels: Option<MergeLabels>,
        options: Option<TreeMergeOptions>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<TreeMergeOutcome> {
        let ancestor = ObjectSpec::extract(ancestor_tree)?;
        let ours = ObjectSpec::extract(our_tree)?;
        let theirs = ObjectSpec::extract(their_tree)?;
        let metadata = Arc::new(Mutex::new(None));
        let result = metadata.clone();
        let editor = TreeEditor::from_factory(py, self.handle.clone(), progress, cancel, move |repo| {
            let options = options
                .map(|v| Ok(v.inner))
                .unwrap_or_else(|| repo.tree_merge_options())
                .map_err(to_py)?;
            let outcome = repo
                .merge_trees(
                    ancestor.resolve(repo)?,
                    ours.resolve(repo)?,
                    theirs.resolve(repo)?,
                    labels.unwrap_or_default().native(),
                    options,
                )
                .map_err(to_py)?;
            *result.lock().map_err(to_py)? = Some((outcome.conflicts, outcome.failed_on_first_unresolved_conflict));
            Ok(outcome.tree)
        })?;
        let (conflicts, failed) = metadata
            .lock()
            .map_err(to_py)?
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("merge returned no metadata"))?;
        Ok(TreeMergeOutcome {
            tree: Py::new(py, editor)?,
            conflicts: Arc::new(conflicts),
            failed,
        })
    }
    #[pyo3(signature=(our_commit,their_commit,labels=None,options=None,*,progress=None,cancel=None))]
    #[allow(clippy::too_many_arguments)]
    fn merge_commits(
        &self,
        py: Python<'_>,
        our_commit: &Bound<'_, PyAny>,
        their_commit: &Bound<'_, PyAny>,
        labels: Option<MergeLabels>,
        options: Option<CommitMergeOptions>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<CommitMergeOutcome> {
        let ours = ObjectSpec::extract(our_commit)?;
        let theirs = ObjectSpec::extract(their_commit)?;
        let metadata = Arc::new(Mutex::new(None));
        let result = metadata.clone();
        let editor = TreeEditor::from_factory(py, self.handle.clone(), progress, cancel, move |repo| {
            let options = options
                .map(|v| Ok(v.inner))
                .unwrap_or_else(|| repo.tree_merge_options().map(Into::into))
                .map_err(to_py)?;
            let outcome = repo
                .merge_commits(
                    ours.resolve(repo)?,
                    theirs.resolve(repo)?,
                    labels.unwrap_or_default().native(),
                    options,
                )
                .map_err(to_py)?;
            *result.lock().map_err(to_py)? = Some((
                outcome.tree_merge.conflicts,
                outcome.tree_merge.failed_on_first_unresolved_conflict,
                outcome.merge_base_tree_id,
                outcome.merge_bases,
                outcome.virtual_merge_bases,
            ));
            Ok(outcome.tree_merge.tree)
        })?;
        let (conflicts, failed, base, bases, virtuals) = metadata
            .lock()
            .map_err(to_py)?
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("merge returned no metadata"))?;
        Ok(CommitMergeOutcome {
            tree_merge: Py::new(
                py,
                TreeMergeOutcome {
                    tree: Py::new(py, editor)?,
                    conflicts: Arc::new(conflicts),
                    failed,
                },
            )?,
            merge_base_tree_id: ObjectId { inner: base },
            merge_bases: bases.map(|v| v.into_iter().map(|inner| ObjectId { inner }).collect()),
            virtual_merge_bases: virtuals.into_iter().map(|inner| ObjectId { inner }).collect(),
        })
    }
    #[pyo3(signature=(merge_bases,options=None,*,progress=None,cancel=None))]
    fn virtual_merge_base(
        &self,
        py: Python<'_>,
        merge_bases: &Bound<'_, PyAny>,
        options: Option<TreeMergeOptions>,
        progress: Option<&Progress>,
        cancel: Option<&CancellationToken>,
    ) -> PyResult<VirtualMergeBase> {
        let bases = merge_bases
            .try_iter()?
            .map(|v| ObjectSpec::extract(&v?))
            .collect::<PyResult<Vec<_>>>()?;
        let handle = self.handle.clone();
        runtime::run(py, "virtual merge base", progress, cancel, move |_| {
            handle.with(|repo| {
                let ids = bases.iter().map(|v| v.resolve(repo)).collect::<PyResult<Vec<_>>>()?;
                let options = options
                    .map(|v| Ok(v.inner))
                    .unwrap_or_else(|| repo.tree_merge_options())
                    .map_err(to_py)?;
                repo.virtual_merge_base(ids, options)
                    .map(|out| VirtualMergeBase {
                        commit_id: ObjectId {
                            inner: out.commit_id.detach(),
                        },
                        tree_id: ObjectId {
                            inner: out.tree_id.detach(),
                        },
                        virtual_merge_bases: out
                            .virtual_merge_bases
                            .into_iter()
                            .map(|v| ObjectId { inner: v.detach() })
                            .collect(),
                    })
                    .map_err(to_py)
            })
        })?
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TreatAsUnresolved>()?;
    m.add_class::<TreeMergeOptions>()?;
    m.add_class::<CommitMergeOptions>()?;
    m.add_class::<MergeLabels>()?;
    m.add_class::<MergeConflict>()?;
    m.add_class::<MergeIndexEntry>()?;
    m.add_class::<ContentMerge>()?;
    m.add_class::<TreeMergeOutcome>()?;
    m.add_class::<CommitMergeOutcome>()?;
    m.add_class::<VirtualMergeBase>()?;
    Ok(())
}
