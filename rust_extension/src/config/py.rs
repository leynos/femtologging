//! Python bindings for configuration builders.

use pyo3::{exceptions::PyValueError, prelude::*, Bound};

use crate::{
    handlers::{FileHandlerBuilder, StreamHandlerBuilder},
    macros::AsPyDict,
};

use super::types::{ConfigError, HandlerBuilder};

impl AsPyDict for HandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        match self {
            Self::Stream(b) => b.as_pydict(py),
            Self::File(b) => b.as_pydict(py),
        }
    }
}

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        PyValueError::new_err(err.to_string())
    }
}

impl<'py> FromPyObject<'py> for HandlerBuilder {
    fn extract_bound(obj: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(b) = obj.extract::<StreamHandlerBuilder>() {
            Ok(HandlerBuilder::Stream(b))
        } else if let Ok(b) = obj.extract::<FileHandlerBuilder>() {
            Ok(HandlerBuilder::File(b))
        } else {
            let ty = obj
                .get_type()
                .name()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "<unknown>".into());
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                "builder must be StreamHandlerBuilder or FileHandlerBuilder (got Python type: {ty})"
            )))
        }
    }
}
