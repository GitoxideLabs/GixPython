use pyo3::{create_exception, exceptions::PyException};

create_exception!(_gix, Error, PyException, "A native Git operation failed.");
create_exception!(
    _gix,
    FeatureUnavailableError,
    Error,
    "This build does not include the requested feature."
);

pub fn to_py(error: impl std::error::Error) -> pyo3::PyErr {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str("\nCaused by: ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    Error::new_err(message)
}
