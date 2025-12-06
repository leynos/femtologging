//! Python bindings for configuration builders.

use pyo3::{
    Bound,
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
};

use crate::handlers::{
    FileHandlerBuilder, RotatingFileHandlerBuilder, SocketHandlerBuilder, StreamHandlerBuilder,
};
use crate::python::fq_py_type;

use super::types::HandlerBuilder;
use crate::config::ConfigError;

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::UnknownIds(ids) => {
                use pyo3::types::PyTuple;
                Python::with_gil(|py| match PyTuple::new(py, ids) {
                    Ok(tup) => PyErr::new::<PyKeyError, _>(Py::<PyTuple>::from(tup)),
                    Err(cause) => {
                        let key_err = PyErr::new::<PyKeyError, _>("unknown handler identifiers");
                        key_err.set_cause(py, Some(cause));
                        key_err
                    }
                })
            }
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
        } else if let Ok(b) = obj.extract::<RotatingFileHandlerBuilder>() {
            Ok(b.into())
        } else if let Ok(b) = obj.extract::<SocketHandlerBuilder>() {
            Ok(b.into())
        } else {
            let fq = fq_py_type(obj);
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "builder must be StreamHandlerBuilder, FileHandlerBuilder, RotatingFileHandlerBuilder, or SocketHandlerBuilder (got Python type: {fq})"
            )))
        }
    }
}
