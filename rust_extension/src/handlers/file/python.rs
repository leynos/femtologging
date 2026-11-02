//! Python bindings for the file-based logging handler.
//!
//! `FemtoFileHandler` is exposed to Python through these `#[pymethods]`
//! definitions. Argument parsing and Python exception construction stay on the
//! interpreter side, and every wait on the worker runs with the interpreter
//! lock released.

use pyo3::prelude::*;

use super::{
    DEFAULT_CHANNEL_CAPACITY, FemtoFileHandler, HandlerConfig, io_utils::open_log_file, policy,
    validate_params,
};
use crate::{formatter::DefaultFormatter, handler::FemtoHandlerTrait, log_record::FemtoLogRecord};

#[pymethods]
impl FemtoFileHandler {
    /// Create a file handler writing to `path`.
    ///
    /// Python usage:
    ///   `FemtoFileHandler(path, capacity=DEFAULT_CHANNEL_CAPACITY,`
    ///   `flush_interval=1, policy="drop")`
    ///
    /// - `capacity` must be greater than zero.
    /// - `flush_interval` must be greater than zero.
    /// - `policy` is one of: `"drop"`, `"block"`, or `"timeout:N"` (N > 0).
    #[new]
    #[pyo3(
        text_signature = "(path, capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop')"
    )]
    #[pyo3(signature=(
        path,
        capacity = DEFAULT_CHANNEL_CAPACITY as isize,
        flush_interval = 1,
        policy = "drop"
    ))]
    fn py_new(
        path: String,
        capacity: isize,
        flush_interval: isize,
        policy: &str,
    ) -> PyResult<Self> {
        let overflow_policy = policy::parse_policy_string(policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        let (capacity, flush_interval) = validate_params(capacity, flush_interval)?;
        let handler_cfg = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let file = open_log_file(&path)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(err.to_string()))?;
        Ok(FemtoFileHandler::from_file(
            file,
            DefaultFormatter,
            handler_cfg,
        ))
    }

    /// Parses a Python record, enqueues it according to the configured
    /// overflow policy, and maps delivery failures to a Python runtime error.
    ///
    /// The wait for queue capacity runs with the interpreter lock released, so
    /// a `"block"` or `"timeout:N"` policy parks only the calling thread. See
    /// the "Concurrency" section of the user guide for the full contract.
    #[pyo3(name = "handle")]
    // @codescene(disable:"Excess Number of Function Arguments") PyO3 token and stable Python API (2026-10-28).
    fn py_handle(&self, py: Python<'_>, logger: &str, level: &str, message: &str) -> PyResult<()> {
        // Parsing raises `ValueError`, so it must happen while the lock is
        // still held; only the enqueue below can wait on the worker.
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        let record = FemtoLogRecord::new(logger, parsed_level, message);
        py.detach(|| <Self as FemtoHandlerTrait>::handle(self, record))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush queued log records to the underlying file without closing the
    /// handler.
    ///
    /// Uses a fixed 1-second timeout, and waits with the interpreter lock
    /// released.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush within the
    ///     timeout.
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
    ///
    /// The GIL is released while waiting so a Python callable formatter can
    /// run on the worker thread.
    #[pyo3(name = "flush")]
    fn py_flush(&self, py: Python<'_>) -> bool {
        py.detach(|| self.flush())
    }

    /// Closes the handler, signals worker shutdown, and waits briefly for
    /// completion before joining the worker when it reports that it has exited.
    ///
    /// The GIL is released while waiting so a Python callable formatter can
    /// finish on the worker thread. Only a shared PyO3 borrow is needed, so a
    /// close can start while another thread still holds the handler in a
    /// detached flush call. The Python borrow is released before the wait, so a
    /// concurrent call reaches the closed handler instead of being rejected by
    /// PyO3. Closing does not cancel a `handle` call that is already waiting for
    /// queue capacity; that call returns only when the worker makes room or the
    /// channel closes. See the user guide for the shutdown contract.
    #[pyo3(name = "close")]
    fn py_close(slf: Py<Self>, py: Python<'_>) {
        let shutdown = {
            let handler = slf.borrow(py);
            handler.take_shutdown()
        };
        py.detach(move || shutdown.wait());
    }
}
