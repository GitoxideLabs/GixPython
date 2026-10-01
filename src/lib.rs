//! Python bindings to the repository API of gitoxide.

use pyo3::prelude::*;

mod error;
mod runtime;

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
