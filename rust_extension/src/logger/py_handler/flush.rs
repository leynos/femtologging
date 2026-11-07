//! Flush support for handlers registered through Python.
//!
//! `flush_handlers` must reach the handler behind the Python wrapper, not just
//! the dispatch shard. A built-in handler is flushed through its native
//! implementation so the wait releases the GIL; a user-defined handler is
//! asked to flush through its own method.

use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

use super::{PyHandler, map_py_err};

impl PyHandler {
    /// Flush the wrapped handler, attaching the interpreter only to reach it.
    ///
    /// A built-in handler is flushed through its native implementation, which
    /// releases the GIL for the wait. A user-defined handler is asked to flush
    /// through its own method. The interpreter is attached only to reach the
    /// handler; the native wait itself runs detached.
    pub(super) fn flush_registered_handler(&self) -> bool {
        Python::try_attach(|py| match self.native_handler {
            Some(native_handler) => native_handler.flush(py, self.obj.bind(py)),
            None => self.call_python_flush(py),
        })
        .unwrap_or(true)
    }

    /// Flush a handler that is not one of the built-in native handlers.
    ///
    /// Python handlers only have to implement `handle`, so a missing (or
    /// non-callable) `flush` means there is nothing to flush and counts as
    /// success. A `flush` that raises is reported as a failure; one that
    /// returns `None` or another non-boolean value counts as success.
    fn call_python_flush(&self, py: Python<'_>) -> bool {
        let Ok(method) = self.obj.bind(py).getattr("flush") else {
            return true;
        };
        if !method.is_callable() {
            return true;
        }
        match method.call0() {
            Ok(result) => result.extract::<bool>().unwrap_or(true),
            Err(err) => {
                map_py_err(py, err, "flush");
                false
            }
        }
    }
}
