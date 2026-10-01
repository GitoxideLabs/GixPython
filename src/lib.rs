//! Python bindings to the repository API of gitoxide.

use pyo3::prelude::*;

#[cfg(feature = "attributes")]
mod attributes;
#[cfg(feature = "blame")]
mod blame;
#[cfg(feature = "merge")]
mod blob_merge;
mod branch;
mod config;
mod config_queries;
#[cfg(feature = "blob-diff")]
mod diff;
#[cfg(feature = "blob-diff")]
mod diff_options;
#[cfg(feature = "dirwalk")]
mod dirwalk;
mod error;
#[cfg(feature = "attributes")]
mod filter;
mod history;
#[cfg(feature = "index")]
mod index;
#[cfg(feature = "merge")]
mod merge;
#[cfg(feature = "network")]
mod network;
#[cfg(feature = "notes")]
mod notes;
mod objects;
#[cfg(feature = "attributes")]
mod pathspec;
mod references;
mod repository;
#[cfg(feature = "revision")]
mod revision;
mod runtime;
#[cfg(feature = "status")]
mod status;
#[cfg(feature = "attributes")]
mod submodule;
mod types;
mod worktree;

const GIX_REVISION: &str = "f819565c2c4c56619c4888acef6cf3b8144cbccb";

/// Return the compile-time capabilities of this installation.
#[pyfunction]
fn build_features() -> Vec<&'static str> {
    let mut out = vec!["parallel"];
    macro_rules! feature {
        ($($name:literal),* $(,)?) => { $(if cfg!(feature = $name) { out.push($name); })* };
    }
    feature!(
        "sha1",
        "sha256",
        "max-performance",
        "revision",
        "revparse-regex",
        "index",
        "attributes",
        "dirwalk",
        "status",
        "blob-diff",
        "merge",
        "blame",
        "notes",
        "mailmap",
        "signing",
        "worktree-mutation",
        "worktree-stream",
        "worktree-archive",
        "archive-tar",
        "archive-tar-gz",
        "archive-zip",
        "network",
        "http",
        "https",
        "tracing",
        "tracing-detail"
    );
    out
}

#[pymodule(gil_used = false)]
fn _gix(m: &Bound<'_, PyModule>) -> PyResult<()> {
    types::register(m)?;
    repository::register(m)?;
    objects::register(m)?;
    config::register(m)?;
    history::register(m)?;
    config_queries::register(m)?;
    #[cfg(feature = "attributes")]
    attributes::register(m)?;
    #[cfg(feature = "attributes")]
    filter::register(m)?;
    #[cfg(feature = "attributes")]
    submodule::register(m)?;
    references::register(m)?;
    #[cfg(feature = "index")]
    index::register(m)?;
    #[cfg(feature = "blob-diff")]
    diff_options::register(m)?;
    #[cfg(feature = "blob-diff")]
    diff::register(m)?;
    worktree::register(m)?;
    #[cfg(feature = "attributes")]
    pathspec::register(m)?;
    #[cfg(feature = "dirwalk")]
    dirwalk::register(m)?;
    #[cfg(feature = "status")]
    status::register(m)?;
    #[cfg(feature = "blame")]
    blame::register(m)?;
    #[cfg(feature = "merge")]
    merge::register(m)?;
    #[cfg(feature = "merge")]
    blob_merge::register(m)?;
    #[cfg(feature = "notes")]
    notes::register(m)?;
    #[cfg(feature = "network")]
    network::register(m)?;
    #[cfg(feature = "revision")]
    revision::register(m)?;
    m.add_class::<runtime::Progress>()?;
    m.add_class::<runtime::CancellationToken>()?;
    m.add("CancelledError", m.py().get_type::<runtime::CancelledError>())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("__gix_revision__", GIX_REVISION)?;
    m.add("Error", m.py().get_type::<error::Error>())?;
    m.add(
        "FeatureUnavailableError",
        m.py().get_type::<error::FeatureUnavailableError>(),
    )?;
    m.add_function(wrap_pyfunction!(build_features, m)?)?;
    Ok(())
}
