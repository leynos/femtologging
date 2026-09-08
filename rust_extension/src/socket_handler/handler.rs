//! Public handler type exported by the crate.
#[cfg(feature = "python")]
#[path = "python_bindings.rs"]
mod python_bindings;

use std::{thread, time::Duration};

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

#[cfg_attr(feature = "python", pyo3::pyclass)]
/// Handler forwarding records to a socket using `MessagePack` framing.
pub struct FemtoSocketHandler {
    tx: Option<crossbeam_channel::Sender<SocketCommand>>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    warner: RateLimitedWarner,
    flush_timeout: Duration,
}

impl FemtoSocketHandler {
    /// Construct a handler targeting the provided transport with default configuration.
    #[must_use]
    pub fn new(transport: SocketTransport) -> Self {
        Self::with_config(SocketHandlerConfig::default().with_transport(transport))
    }

    /// Construct the handler from a configuration object.
    #[must_use]
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
        self.tx.clone()
    }

    fn request_shutdown(&mut self) {
        let Some(tx) = self.tx.take() else {
            return;
        };
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        if tx.send(SocketCommand::Shutdown(ack_tx)).is_err() {
            return;
        }
        let Ok(()) = ack_rx.recv_timeout(self.flush_timeout) else {
            // Shutdown remains bounded even when the worker cannot acknowledge it.
            return;
        };
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
            .field("tx", &self.tx)
            .field("handle", &self.handle)
            .field("warner", &"RateLimitedWarner")
            .field("flush_timeout", &self.flush_timeout)
            .finish()
    }
}
