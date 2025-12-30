//! Public handler type exported by the crate.

use std::{thread, time::Duration};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use parking_lot::Mutex;

use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
};

use super::{
    config::HTTPHandlerConfig,
    worker::{HTTPCommand, enqueue_record, flush_queue, spawn_worker},
};

#[cfg_attr(feature = "python", pyclass)]
/// Handler forwarding records to an HTTP endpoint.
///
/// Supports URL-encoded form data (CPython parity) and JSON serialization.
/// Uses exponential backoff for transient failures (5xx, 429, network errors)
/// and drops records on permanent failures (4xx except 429).
pub struct FemtoHTTPHandler {
    tx: Option<crossbeam_channel::Sender<HTTPCommand>>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    warner: RateLimitedWarner,
    /// Timeout for flush and shutdown operations.
    ///
    /// Derived from `write_timeout` in the configuration: a flush or graceful
    /// shutdown should complete within the same time bounds as a single HTTP
    /// request.
    flush_timeout: Duration,
}

impl FemtoHTTPHandler {
    /// Construct the handler from a configuration object.
    pub fn with_config(config: HTTPHandlerConfig) -> Self {
        let flush_timeout = config.write_timeout;
        let warner = RateLimitedWarner::new(config.warn_interval);
        let (tx, handle) = spawn_worker(config);
        Self {
            tx: Some(tx),
            handle: Mutex::new(Some(handle)),
            warner,
            flush_timeout,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        <Self as FemtoHandlerTrait>::flush(self)
    }

    /// Close the handler and wait for the worker to exit.
    pub fn close(&mut self) {
        self.request_shutdown();
        self.join_worker();
    }

    fn sender(&self) -> Option<crossbeam_channel::Sender<HTTPCommand>> {
        self.tx.as_ref().cloned()
    }

    fn request_shutdown(&mut self) {
        let Some(tx) = self.tx.take() else {
            return;
        };
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        if tx.send(HTTPCommand::Shutdown(ack_tx)).is_err() {
            return;
        }
        let _ = ack_rx.recv_timeout(self.flush_timeout);
    }

    fn join_worker(&mut self) {
        let Some(handle) = self.handle.lock().take() else {
            return;
        };
        if handle.join().is_err() {
            log::warn!("FemtoHTTPHandler: worker thread panicked");
        }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoHTTPHandler {
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = crate::level::FemtoLevel::parse_or_warn(level);
        self.handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool {
        self.flush()
    }

    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
    }
}

impl FemtoHandlerTrait for FemtoHTTPHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = self.sender() else {
            self.warner.record_drop();
            self.warner.warn_if_due(|count| {
                log::warn!("FemtoHTTPHandler dropped {count} records after shutdown");
            });
            return Err(HandlerError::Closed);
        };
        enqueue_record(&tx, record, &self.warner)
    }

    fn flush(&self) -> bool {
        let Some(tx) = self.sender() else {
            return false;
        };
        self.warner.flush(|count| {
            log::warn!("FemtoHTTPHandler dropped {count} records in the last interval");
        });
        flush_queue(&tx, self.flush_timeout)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Drop for FemtoHTTPHandler {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for FemtoHTTPHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoHTTPHandler")
            .field("flush_timeout", &self.flush_timeout)
            .finish()
    }
}
