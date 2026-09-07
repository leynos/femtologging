//! Python method wrappers for [`super::FemtoFileHandler`].
//!
//! Keeping the `PyO3` expansion in this private module leaves the file handler's
//! Rust implementation and direct API in the parent module.

use pyo3::prelude::*;

use super::{
    DEFAULT_CHANNEL_CAPACITY,
    DefaultFormatter,
    FemtoFileHandler,
    FemtoHandlerTrait,
    FemtoLogRecord,
    HandlerConfig,
    open_log_file,
    policy,
    validate_params,
};

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
        text_signature = "(path, capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, \
                          policy='drop')"
    )]
    #[pyo3(signature=(
        path,
        capacity = DEFAULT_CHANNEL_CAPACITY.cast_signed(),
        flush_interval = 1,
        policy = "drop"
    ))]
    fn py_new(path: &str, capacity: isize, flush_interval: isize, policy: &str) -> PyResult<Self> {
        let overflow_policy = policy::parse_policy_string(policy)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        let (validated_capacity, validated_flush_interval) =
            validate_params(capacity, flush_interval)?;
        let handler_cfg = HandlerConfig {
            capacity: validated_capacity,
            flush_interval: validated_flush_interval,
            overflow_policy,
        };
        let file = open_log_file(path)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(err.to_string()))?;
        Ok(Self::from_file(file, DefaultFormatter, handler_cfg))
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        <Self as FemtoHandlerTrait>::handle(
            self,
            FemtoLogRecord::new(logger, parsed_level, message),
        )
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush queued log records to the underlying file without closing the
    /// handler.
    ///
    /// Uses a fixed 1-second timeout.
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
    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool { self.flush() }

    #[pyo3(name = "close")]
    fn py_close(&mut self) { self.close(); }
}
