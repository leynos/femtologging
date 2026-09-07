//! Python bindings for configuration builders.

use pyo3::{
    Borrowed,
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
};

use crate::handlers::{
    FileHandlerBuilder, RotatingFileHandlerBuilder, SocketHandlerBuilder, StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
};
use crate::python::fq_py_type;

use super::types::HandlerBuilder;
use crate::config::ConfigError;

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::UnknownIds(ids) => {
                use pyo3::types::PyTuple;
                Python::attach(|py| match PyTuple::new(py, ids) {
                    Ok(tup) => Self::new::<PyKeyError, _>(Py::<PyTuple>::from(tup)),
                    Err(cause) => {
                        let key_err = Self::new::<PyKeyError, _>("unknown handler identifiers");
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

impl<'a, 'py> FromPyObject<'a, 'py> for HandlerBuilder {
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> Result<Self, Self::Error> {
        obj.extract::<StreamHandlerBuilder>()
            .map(Self::from)
            .or_else(|_| obj.extract::<FileHandlerBuilder>().map(Self::from))
            .or_else(|_| {
                obj.extract::<RotatingFileHandlerBuilder>()
                    .map(Self::from)
            })
            .or_else(|_| {
                obj.extract::<TimedRotatingFileHandlerBuilder>()
                    .map(Self::from)
            })
            .or_else(|_| obj.extract::<SocketHandlerBuilder>().map(Self::from))
            .map_err(|_| {
                let fq = fq_py_type(&obj.to_owned());
                pyo3::exceptions::PyTypeError::new_err(format!(
                    "builder must be StreamHandlerBuilder, FileHandlerBuilder, RotatingFileHandlerBuilder, TimedRotatingFileHandlerBuilder, or SocketHandlerBuilder (got Python type: {fq})"
                ))
            })
    }
}
