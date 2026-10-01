use pyo3::{create_exception, exceptions::PyException};

create_exception!(_gix, Error, PyException, "A native Git operation failed.");
create_exception!(
    _gix,
    FeatureUnavailableError,
    Error,
    "This build does not include the requested feature."
);
