//! Background worker thread and queue management for the FemtoLogger.
//!
//! The worker accepts queued log records, dispatches them to handlers, and
//! coordinates graceful shutdown when the logger drops.

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, select, Receiver, Sender};
use log::warn;

use crate::handler::FemtoHandlerTrait;
use crate::log_record::FemtoLogRecord;

/// Default capacity for the bounded channel feeding the worker thread.
pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Record queued for processing by the worker thread.
pub struct QueuedRecord {
    pub record: FemtoLogRecord,
    pub handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
}

/// Handle to the worker thread and its communication channels.
pub(crate) struct WorkerParts {
    pub(crate) tx: Sender<QueuedRecord>,
    pub(crate) shutdown_tx: Sender<()>,
    pub(crate) handle: JoinHandle<()>,
}

/// Spawn the worker thread and return its communication primitives.
pub(crate) fn spawn_worker() -> WorkerParts {
    let (tx, rx) = bounded::<QueuedRecord>(DEFAULT_CHANNEL_CAPACITY);
    let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
    let handle = thread::spawn(move || worker_thread_loop(rx, shutdown_rx));

    WorkerParts {
        tx,
        shutdown_tx,
        handle,
    }
}

/// Process a single `FemtoLogRecord` by dispatching it to all handlers.
pub(crate) fn handle_log_record(job: QueuedRecord) {
    for h in job.handlers.iter() {
        if let Err(err) = h.handle(job.record.clone()) {
            warn!("FemtoLogger: handler reported an error: {err}");
        }
    }
}

/// Drain any remaining records once a shutdown signal is received.
///
/// Consumes all messages still available on `rx` and dispatches them
/// through the provided `handlers`. This ensures no log records are lost
/// during shutdown.
pub(crate) fn drain_remaining_records(rx: &Receiver<QueuedRecord>) {
    while let Ok(job) = rx.try_recv() {
        handle_log_record(job);
    }
}

/// Main loop executed by the logger's worker thread.
///
/// Waits on either incoming log records or a shutdown signal using
/// `select!`. Each received record is forwarded to `handle_log_record`.
/// When a shutdown signal arrives, any queued records are drained before
/// the thread exits.
pub(crate) fn worker_thread_loop(rx: Receiver<QueuedRecord>, shutdown_rx: Receiver<()>) {
    loop {
        select! {
            recv(rx) -> rec => match rec {
                Ok(job) => handle_log_record(job),
                Err(_) => break,
            },
            recv(shutdown_rx) -> _ => {
                drain_remaining_records(&rx);
                break;
            }
        }
    }
}
