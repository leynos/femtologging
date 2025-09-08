//! Python bindings for configuration builders.

use pyo3::{exceptions::PyValueError, prelude::*, Bound};

use crate::handlers::{FileHandlerBuilder, StreamHandlerBuilder};

use super::types::{ConfigError, HandlerBuilder};

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        PyValueError::new_err(err.to_string())
    }
}

impl<'py> FromPyObject<'py> for HandlerBuilder {
    fn extract_bound(obj: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(b) = obj.extract::<StreamHandlerBuilder>() {
            Ok(b.into())
        } else if let Ok(b) = obj.extract::<FileHandlerBuilder>() {
            Ok(b.into())
        } else {
            let ty = obj
                .get_type()
                .name()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "<unknown>".into());
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "builder must be StreamHandlerBuilder or FileHandlerBuilder (got Python type: {ty})"
            )))
        }
    }
}
