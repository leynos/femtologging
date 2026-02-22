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
    SocketTransport,
    config::SocketHandlerConfig,
    worker::{SocketCommand, enqueue_record, flush_queue, spawn_worker},
};

#[cfg_attr(feature = "python", pyclass)]
/// Handler forwarding records to a socket using MessagePack framing.
pub struct FemtoSocketHandler {
    tx: Option<crossbeam_channel::Sender<SocketCommand>>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    warner: RateLimitedWarner,
    flush_timeout: Duration,
}

impl FemtoSocketHandler {
    /// Construct a handler targeting the provided transport with default configuration.
    pub fn new(transport: SocketTransport) -> Self {
        Self::with_config(SocketHandlerConfig::default().with_transport(transport))
    }

    /// Construct the handler from a configuration object.
    pub fn with_config(config: SocketHandlerConfig) -> Self {
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

    fn sender(&self) -> Option<crossbeam_channel::Sender<SocketCommand>> {
        self.tx.as_ref().cloned()
    }

    fn request_shutdown(&mut self) {
        let Some(tx) = self.tx.take() else {
            return;
        };
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        if tx.send(SocketCommand::Shutdown(ack_tx)).is_err() {
            return;
        }
        let _ = ack_rx.recv_timeout(self.flush_timeout);
    }

    fn join_worker(&mut self) {
        let Some(handle) = self.handle.lock().take() else {
            return;
        };
        if handle.join().is_err() {
            log::warn!("FemtoSocketHandler: worker thread panicked");
        }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoSocketHandler {
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        self.handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush pending log records without shutting down the worker thread.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush command within the
    ///     1-second timeout.
    ///     ``False`` when the handler has already been closed, the command
    ///     cannot be delivered to the worker, or the worker fails to
    ///     acknowledge before the timeout elapses.
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
        self.flush()
    }

    /// Close the handler and wait for the worker thread to finish.
    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
    }
}

impl FemtoHandlerTrait for FemtoSocketHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = self.sender() else {
            self.warner.record_drop();
            self.warner.warn_if_due(|count| {
                log::warn!("FemtoSocketHandler dropped {count} records after shutdown");
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
            log::warn!("FemtoSocketHandler dropped {count} records in the last interval");
        });
        flush_queue(&tx, self.flush_timeout)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Drop for FemtoSocketHandler {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for FemtoSocketHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoSocketHandler")
            .field("flush_timeout", &self.flush_timeout)
            .finish()
    }
}
