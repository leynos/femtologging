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
            ConfigError::UnknownIds(ids) => {
                use pyo3::types::PyTuple;
                Python::with_gil(|py| {
                    let tup: Py<PyTuple> = PyTuple::new(py, ids).unwrap().into();
                    PyErr::new::<PyKeyError, _>(tup)
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
        } else {
            let fq = fq_py_type(obj);
            Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "builder must be StreamHandlerBuilder or FileHandlerBuilder (got Python type: {fq})"
            )))
        }
    }
}
