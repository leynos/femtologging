//! Python-facing methods for [`FemtoSocketHandler`].
//!
//! These three wrappers only translate Python arguments and release the GIL
//! around the Rust lifecycle the handler already owns, so they change when the
//! Python surface changes and not when the worker protocol does. Keeping them
//! here also holds the handler module inside the per-module size limit.
//!
//! The whole module is compiled only under the `python` feature; the Rust
//! handler and its worker build and test without an interpreter.

use pyo3::prelude::*;

use super::FemtoSocketHandler;
use crate::handler::FemtoHandlerTrait;
use crate::log_record::FemtoLogRecord;

#[pymethods]
impl FemtoSocketHandler {
    /// Convert Python arguments to a record and enqueue it for delivery.
    ///
    /// Example: `handler.handle("app", "INFO", "started")` queues one record.
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        self.handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush pending log records without shutting down the worker thread.
    ///
    /// The flush timeout equals the ``write_timeout`` configured on the
    /// handler (default: 1 second).
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush within the
    ///     configured timeout.
    ///     ``False`` when the handler has already been closed, the
    ///     internal channel to the worker has been dropped, or the worker
    ///     does not acknowledge before the timeout elapses.
    ///
    /// Examples
    /// --------
    /// >>> handler.flush()
    /// True
    /// >>> handler.close()
    /// >>> handler.flush()
    /// False
    #[pyo3(name = "flush")]
    fn py_flush(&self, py: Python<'_>) -> bool {
        crate::py_detach::detach_for_wait(py, || self.flush())
    }

    /// Close the handler, waiting for the worker at most ``write_timeout``.
    ///
    /// A worker that acknowledges the shutdown within ``write_timeout`` is
    /// joined. One that does not, typically because it is stuck writing to an
    /// unresponsive collector, is abandoned with a warning rather than waited
    /// for, and the records still queued are discarded rather than written.
    ///
    /// The wait runs with the GIL released, so a close that spends its budget
    /// waiting does not hold the interpreter while it does.
    #[pyo3(name = "close")]
    fn py_close(&mut self, py: Python<'_>) {
        crate::py_detach::detach_for_wait(py, || self.close());
    }
}
