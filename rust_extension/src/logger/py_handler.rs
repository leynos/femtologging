//! Python handler integration helpers for the FemtoLogger module.
//!
//! This module validates Python handler objects and adapts them to the
//! [`FemtoHandlerTrait`] so Rust loggers can drive them safely.

use std::any::Any;

use log::warn;
use pyo3::prelude::*;
use pyo3::{Py, PyAny};

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

/// Validate that the provided Python object exposes a callable `handle` method.
pub(crate) fn validate_handler(obj: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = obj.py();
    let handle = obj.getattr("handle").map_err(|err| {
        if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
            pyo3::exceptions::PyTypeError::new_err(
                "handler must implement a callable 'handle' method",
            )
        } else {
            err
        }
    })?;
    if handle.is_callable() {
        Ok(())
    } else {
        let attr_type = handle
            .get_type()
            .name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        let handler_repr = obj
            .repr()
            .map(|r| r.to_string())
            .unwrap_or_else(|_| "<unrepresentable>".to_string());
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "'handler.handle' is not callable (type: {attr_type}, handler: {handler_repr})",
        )))
    }
}

/// Wrapper allowing Python handler objects to be used by the logger.
pub(crate) struct PyHandler {
    pub(crate) obj: Py<PyAny>,
}

impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        Python::with_gil(|py| {
            match self.obj.call_method1(
                py,
                "handle",
                (&record.logger, &record.level, &record.message),
            ) {
                Ok(_) => Ok(()),
                Err(err) => {
                    let message = err.to_string();
                    err.print(py);
                    warn!("PyHandler: error calling handle");
                    Err(HandlerError::Message(format!(
                        "python handler raised an exception: {message}"
                    )))
                }
            }
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
