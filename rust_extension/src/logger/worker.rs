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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::{FemtoHandlerTrait, HandlerError};
    use parking_lot::Mutex;
    use std::any::Any;
    use std::sync::Arc;

    #[derive(Default)]
    struct CollectingHandler {
        seen: Arc<Mutex<Vec<FemtoLogRecord>>>,
    }

    impl CollectingHandler {
        fn new() -> Self {
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn records(&self) -> Vec<FemtoLogRecord> {
            self.seen.lock().clone()
        }
    }

    impl FemtoHandlerTrait for CollectingHandler {
        fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
            self.seen.lock().push(record);
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    struct FailingHandler;

    impl FemtoHandlerTrait for FailingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            Err(HandlerError::Message("boom".to_string()))
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn handle_log_record_invokes_all_handlers() {
        let collector = Arc::new(CollectingHandler::new());
        let record = FemtoLogRecord::new("core", "INFO", "msg");
        let queued = QueuedRecord {
            record,
            handlers: vec![
                Arc::new(FailingHandler) as Arc<dyn FemtoHandlerTrait>,
                collector.clone() as Arc<dyn FemtoHandlerTrait>,
            ],
        };

        handle_log_record(queued);

        let collected = collector.records();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].message, "msg");
    }

    #[test]
    fn drain_remaining_records_processes_all_jobs() {
        let collector = Arc::new(CollectingHandler::new());
        let (tx, rx) = bounded(4);

        for idx in 0..3 {
            let message = format!("{idx}");
            tx.send(QueuedRecord {
                record: FemtoLogRecord::new("core", "INFO", &message),
                handlers: vec![collector.clone() as Arc<dyn FemtoHandlerTrait>],
            })
            .expect("queue should accept test record");
        }

        drain_remaining_records(&rx);

        let messages: Vec<String> = collector
            .records()
            .into_iter()
            .map(|rec| rec.message)
            .collect();
        assert_eq!(messages, vec!["0", "1", "2"]);
    }

    #[test]
    fn spawn_worker_processes_and_shuts_down_cleanly() {
        let collector = Arc::new(CollectingHandler::new());
        let WorkerParts {
            tx,
            shutdown_tx,
            handle,
        } = spawn_worker();

        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", "INFO", "alpha"),
            handlers: vec![collector.clone() as Arc<dyn FemtoHandlerTrait>],
        })
        .expect("worker queue should accept record");

        shutdown_tx
            .send(())
            .expect("shutdown channel should accept signal");
        drop(tx);

        handle.join().expect("worker thread should exit normally");

        let messages: Vec<String> = collector
            .records()
            .into_iter()
            .map(|rec| rec.message)
            .collect();
        assert_eq!(messages, vec!["alpha"]);
    }

    #[test]
    fn worker_thread_loop_drains_before_exit() {
        let collector = Arc::new(CollectingHandler::new());
        let (tx, rx) = bounded(4);
        let (shutdown_tx, shutdown_rx) = bounded(1);
        let handler_arc = collector.clone() as Arc<dyn FemtoHandlerTrait>;

        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", "INFO", "first"),
            handlers: vec![handler_arc.clone()],
        })
        .expect("queue should accept first record");
        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", "INFO", "second"),
            handlers: vec![handler_arc],
        })
        .expect("queue should accept second record");

        let worker = std::thread::spawn(move || worker_thread_loop(rx, shutdown_rx));

        shutdown_tx
            .send(())
            .expect("shutdown channel should accept signal");
        drop(tx);
        worker.join().expect("worker loop should exit cleanly");

        let messages: Vec<String> = collector
            .records()
            .into_iter()
            .map(|rec| rec.message)
            .collect();
        assert_eq!(messages, vec!["first", "second"]);
    }
}
