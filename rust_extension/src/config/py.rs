//! Python bindings for configuration builders.

use pyo3::{
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
    Bound,
};

use crate::handlers::{FileHandlerBuilder, StreamHandlerBuilder};

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
            let ty = obj.get_type();
            let module = ty
                .getattr("__module__")
                .and_then(|m| m.extract::<String>())
                .unwrap_or_else(|_| "<unknown>".to_string());
            let qualname = ty
                .getattr("__qualname__")
                .and_then(|n| n.extract::<String>())
                .unwrap_or_else(|_| "<unknown>".to_string());
            let fq = if module == "builtins" {
                qualname.clone()
            } else {
                format!("{module}.{qualname}")
            };
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "builder must be StreamHandlerBuilder or FileHandlerBuilder (got Python type: {fq})"
            )))
        }
    }
}
