//! Python bindings for configuration builders.

use pyo3::{
    Borrowed,
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
};

use crate::handlers::{
    FileHandlerBuilder, HTTPHandlerBuilder, RotatingFileHandlerBuilder, SocketHandlerBuilder,
    StreamHandlerBuilder, TimedRotatingFileHandlerBuilder,
};
use crate::python::fq_py_type;

use super::types::HandlerBuilder;
use crate::config::ConfigError;

/// Converts a borrowed Python handler builder to its Rust enum variant when types match.
type HandlerBuilderExtractor = for<'a, 'py> fn(&Borrowed<'a, 'py, PyAny>) -> Option<HandlerBuilder>;

/// Attempt extraction as one concrete builder type, returning `None` on mismatch.
fn try_extract_builder<T>(obj: &Borrowed<'_, '_, PyAny>) -> Option<HandlerBuilder>
where
    T: for<'a, 'py> FromPyObject<'a, 'py> + Into<HandlerBuilder>,
{
    obj.extract::<T>().ok().map(Into::into)
}

/// Concrete builder types accepted by Python configuration extraction.
const HANDLER_BUILDER_EXTRACTORS: [HandlerBuilderExtractor; 6] = [
    try_extract_builder::<StreamHandlerBuilder>,
    try_extract_builder::<FileHandlerBuilder>,
    try_extract_builder::<RotatingFileHandlerBuilder>,
    try_extract_builder::<TimedRotatingFileHandlerBuilder>,
    try_extract_builder::<SocketHandlerBuilder>,
    try_extract_builder::<HTTPHandlerBuilder>,
];

impl From<ConfigError> for PyErr {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::UnknownIds(ids) => {
                use pyo3::types::PyTuple;
                Python::attach(|py| match PyTuple::new(py, ids) {
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

impl<'a, 'py> FromPyObject<'a, 'py> for HandlerBuilder {
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> Result<Self, Self::Error> {
        HANDLER_BUILDER_EXTRACTORS
            .iter()
            .find_map(|extract| extract(&obj))
            .ok_or_else(|| {
                let fq = fq_py_type(&obj.to_owned());
                pyo3::exceptions::PyTypeError::new_err(format!(
                    "builder must be StreamHandlerBuilder, FileHandlerBuilder, RotatingFileHandlerBuilder, TimedRotatingFileHandlerBuilder, SocketHandlerBuilder, or HTTPHandlerBuilder (got Python type: {fq})"
                ))
            })
    }
}
