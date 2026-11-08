//! Feature-disabled [`PyHandler`] fallback.
//!
//! Without the `python` feature the wrapper cannot inspect its object for a
//! native built-in, so it retains the object only to satisfy the handler
//! trait's dispatch signature and reports every barrier as unconfirmed.
//! Keeping it here leaves the feature-gated implementation in the parent
//! module within its size limit.

use pyo3::PyAny;
use pyo3::prelude::*;
use std::any::Any;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;

use super::map_py_err;

/// Fallback PyHandler when python feature is disabled.
#[cfg(not(feature = "python"))]
pub struct PyHandler {
    /// Python handler object retained by the feature-disabled wrapper.
    pub obj: Py<PyAny>,
}

#[cfg(not(feature = "python"))]
impl PyHandler {
    /// Wrap the Python object without feature-specific capability inspection.
    pub fn new(_py: Python<'_>, obj: Py<PyAny>) -> Self {
        Self { obj }
    }
}

#[cfg(not(feature = "python"))]
impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        Python::attach(|py| {
            self.obj
                .call_method1(
                    py,
                    "handle",
                    (record.logger(), record.level_str(), record.message()),
                )
                .map(|_| ())
                .map_err(|err| map_py_err(py, err, "handle"))
        })
    }

    /// Without the Python feature the wrapper cannot flush its object, so it
    /// reports an unconfirmed barrier rather than claiming work it did not do.
    fn flush_until(&self, _deadline: std::time::Instant) -> bool {
        false
    }

    fn is_python_backed(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
