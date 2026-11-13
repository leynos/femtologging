//! Python bindings for the file-based logging handler.
//!
//! `FemtoFileHandler` is exposed to Python through these `#[pymethods]`
//! definitions. Argument parsing and Python exception construction stay on the
//! interpreter side, and every wait on the worker runs with the interpreter
//! lock released.

use std::sync::Arc;

use pyo3::prelude::*;

use super::{
    DEFAULT_CHANNEL_CAPACITY, FemtoFileHandler, HandlerConfig, classify_enqueue,
    io_utils::open_log_file, policy, validate_params,
};
use crate::handlers::telemetry::{WaitContext, WaitScope, WaitSite};
use crate::{formatter::DefaultFormatter, log_record::FemtoLogRecord};

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
        let (result, timer) =
            WaitScope::start(self.diagnostics()).run(|| py.detach(|| self.enqueue(record)));
        self.record_wait(
            timer,
            WaitSite::new("handler_handle", "file"),
            classify_enqueue(&result),
        );
        result
            .map_err(super::EnqueueFailure::into_handler_error)
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
        let (outcome, timer) =
            WaitScope::start(self.diagnostics()).run(|| py.detach(|| self.flush_outcome()));
        self.record_wait(
            timer,
            WaitSite::new("handler_flush", "file"),
            outcome.into(),
        );
        matches!(outcome, super::FlushOutcome::Acknowledged)
    }

    /// Return bounded counters for the Python-facing `handle()`, `flush()`,
    /// and `close()` waits.
    ///
    /// Returns
    /// -------
    /// dict
    ///     A fixed key set: one counter per outcome, plus
    ///     ``total_wait_micros``, ``max_wait_micros``, and the
    ///     ``waits_in_flight`` gauge. Nothing here carries logger names,
    ///     paths, messages, or raw timeout values, so the set stays bounded
    ///     however much traffic the handler sees.
    ///
    /// Examples
    /// --------
    /// >>> diagnostics = handler.wait_diagnostics()
    /// >>> diagnostics["handle_accepted"]
    /// 3
    /// >>> diagnostics["total_wait_micros"] >= diagnostics["max_wait_micros"]
    /// True
    #[pyo3(name = "wait_diagnostics")]
    fn py_wait_diagnostics(&self) -> std::collections::BTreeMap<&'static str, u64> {
        self.diagnostics().snapshot().to_map()
    }

    /// Report how many records are inside the channel send, for unit tests.
    ///
    /// The public gauge rises when a call enters its wait, which happens before
    /// the sender is cloned, so it cannot prove a record has reached a full
    /// queue. The interleaving tests read this count for that distinction. It
    /// is compiled out of release builds, so it is not part of the shipped API.
    #[cfg(test)]
    #[pyo3(name = "debug_sends_in_flight")]
    fn py_debug_sends_in_flight(&self) -> u64 {
        self.sends_in_flight()
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
        // Capture the counters and the bounded policy bucket while the borrow
        // is held; the shutdown wait itself must not touch the object.
        let (shutdown, diagnostics, policy_bucket) = {
            let handler = slf.borrow(py);
            (
                handler.take_shutdown(),
                Arc::clone(handler.diagnostics()),
                handler.policy_bucket(),
            )
        };
        let (outcome, timer) =
            WaitScope::start(&diagnostics).run(move || py.detach(move || shutdown.wait()));
        timer.finish(
            &diagnostics,
            WaitContext {
                operation: "handler_close",
                handler_kind: "file",
                policy_bucket,
            },
            outcome.into(),
        );
    }
}
