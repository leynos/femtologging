//! `PyO3` wrappers for the rotating handler types and test controls.
//!
//! The parent module owns configuration and rotation logic; this private
//! module contains only Python-facing macro expansions and their adapters.

use pyo3::prelude::*;

use super::{CoreRotatingFileHandler, HandlerOptions, PyRotatingFileHandler, fresh_failure};
use crate::{
    formatter::DefaultFormatter, handler::FemtoHandlerTrait, level::FemtoLevel,
    log_record::FemtoLogRecord,
};

#[pymethods]
impl HandlerOptions {
    #[new]
    #[pyo3(
        text_signature = "(capacity=DEFAULT_CHANNEL_CAPACITY, flush_interval=1, policy='drop', rotation=None)"
    )]
    #[pyo3(signature = (
        capacity = super::DEFAULT_CHANNEL_CAPACITY,
        flush_interval = 1,
        policy = "drop".to_owned(),
        rotation = None,
    ))]
    fn new(
        capacity: usize,
        flush_interval: isize,
        policy: String,
        rotation: Option<(u64, usize)>,
    ) -> PyResult<Self> {
        let (max_bytes, backup_count) = rotation.unwrap_or((0, 0));

        let options = Self {
            capacity,
            flush_interval,
            policy,
            max_bytes,
            backup_count,
        };

        // Validate using the same logic as py_new; discard configs, keep validated opts
        let _ = options.to_configs()?;
        Ok(options)
    }
}

#[pymethods]
impl PyRotatingFileHandler {
    #[new]
    #[pyo3(text_signature = "(path, options=None)")]
    #[pyo3(signature = (path, options = None))]
    fn py_new(path: &str, options: Option<HandlerOptions>) -> PyResult<Self> {
        let opts = options.unwrap_or_default();
        let (handler_cfg, rotation) = opts.to_configs()?;

        CoreRotatingFileHandler::with_capacity_flush_policy(
            path,
            DefaultFormatter,
            handler_cfg,
            rotation,
        )
        .map(Self::from_core)
        .map_err(|err| pyo3::exceptions::PyIOError::new_err(format!("{path}: {err}")))
    }

    /// Expose the configured maximum number of bytes before rotation.
    #[getter]
    const fn max_bytes(&self) -> u64 {
        self.inner.rotation_limits().0
    }

    /// Expose the configured backup count.
    #[getter]
    const fn backup_count(&self) -> usize {
        self.inner.rotation_limits().1
    }

    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = FemtoLevel::parse_py(level)?;
        self.inner
            .handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush queued log records to disk without closing the handler or
    /// triggering a rotation.
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
    fn py_flush(&self) -> bool {
        self.inner.flush()
    }

    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.inner.close();
    }
}

#[pyfunction]
pub fn force_rotating_fresh_failure_for_test(count: usize, reason: Option<&str>) {
    let configured_reason = reason.map_or_else(
        || "python requested failure".to_owned(),
        std::borrow::ToOwned::to_owned,
    );
    fresh_failure::set_forced_fresh_failure(count, configured_reason);
}

#[pyfunction]
pub fn clear_rotating_fresh_failure_for_test() {
    fresh_failure::clear_forced_fresh_failure();
}
