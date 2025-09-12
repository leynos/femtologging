//! Python bindings for configuration builders.

use pyo3::{
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
    Bound,
};

use crate::handlers::{FileHandlerBuilder, StreamHandlerBuilder};
use crate::python::fq_py_type;

use super::types::HandlerBuilder;
use crate::config::ConfigError;

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::UnknownId(id) => PyKeyError::new_err(id),
            ConfigError::LoggerInit(msg) => PyRuntimeError::new_err(msg),
            _ => PyValueError::new_err(err.to_string()),
        }
    }
}

impl<'py> FromPyObject<'py> for HandlerBuilder {
    fn extract_bound(obj: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(b) = obj.extract::<StreamHandlerBuilder>() {
            Ok(b.into())
        } else if let Ok(b) = obj.extract::<FileHandlerBuilder>() {
            Ok(b.into())
        } else {
            let fq = fq_py_type(obj);
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "builder must be StreamHandlerBuilder or FileHandlerBuilder (got Python type: {fq})"
            )))
        }
    }
}
