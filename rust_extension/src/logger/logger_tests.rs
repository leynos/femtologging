//! Unit tests for FemtoLogger.

use super::*;
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Default)]
struct CollectingHandler {
    records: Arc<Mutex<Vec<FemtoLogRecord>>>,
}

impl CollectingHandler {
    fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn collected(&self) -> Vec<FemtoLogRecord> {
        self.records.lock().clone()
    }
}

impl FemtoHandlerTrait for CollectingHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.records.lock().push(record);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Default)]
struct CountingHandler {
    count: Arc<AtomicUsize>,
    first_tx: Option<crossbeam_channel::Sender<()>>,
}

impl CountingHandler {
    fn with_first_signal(first_tx: crossbeam_channel::Sender<()>) -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
            first_tx: Some(first_tx),
        }
    }
}

impl FemtoHandlerTrait for CountingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        if self.count.fetch_add(1, Ordering::SeqCst) == 0 {
            if let Some(first_tx) = &self.first_tx {
                let _ = first_tx.send(());
            }
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn handle_log_record_dispatches() {
    let h1 = Arc::new(CollectingHandler::new());
    let h2 = Arc::new(CollectingHandler::new());
    let record = QueuedRecord {
        record: FemtoLogRecord::new("core", FemtoLevel::Info, "msg"),
        handlers: vec![
            h1.clone() as Arc<dyn FemtoHandlerTrait>,
            h2.clone() as Arc<dyn FemtoHandlerTrait>,
        ],
    };

    FemtoLogger::handle_log_record(record);

    let r1 = h1.collected();
    let r2 = h2.collected();
    assert_eq!(r1.len(), 1);
    assert_eq!(r2.len(), 1);
    assert_eq!(r1[0].message(), "msg");
    assert_eq!(r2[0].message(), "msg");
}

#[test]
fn drain_remaining_records_pulls_all() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let h = Arc::new(CollectingHandler::new());
    for i in 0..3 {
        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", FemtoLevel::Info, &format!("{i}")),
            handlers: vec![h.clone() as Arc<dyn FemtoHandlerTrait>],
        })
        .expect("Failed to send test record");
    }
    drop(tx);

    FemtoLogger::drain_remaining_records(&rx);

    let collected = h.collected();
    let msgs: Vec<&str> = collected.iter().map(|r| r.message()).collect();
    assert_eq!(msgs, vec!["0", "1", "2"]);
}

#[test]
fn worker_thread_loop_processes_and_drains() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
    let h = Arc::new(CollectingHandler::new());

    let thread = std::thread::spawn(move || {
        FemtoLogger::worker_thread_loop(rx, shutdown_rx);
    });

    tx.send(QueuedRecord {
        record: FemtoLogRecord::new("core", FemtoLevel::Info, "one"),
        handlers: vec![h.clone() as Arc<dyn FemtoHandlerTrait>],
    })
    .expect("Failed to send first test record");
    tx.send(QueuedRecord {
        record: FemtoLogRecord::new("core", FemtoLevel::Info, "two"),
        handlers: vec![h.clone() as Arc<dyn FemtoHandlerTrait>],
    })
    .expect("Failed to send second test record");
    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");
    thread.join().expect("Worker thread panicked");

    let collected = h.collected();
    let msgs: Vec<&str> = collected.iter().map(|r| r.message()).collect();
    assert_eq!(msgs, vec!["one", "two"]);
}

#[test]
fn worker_thread_loop_shutdown_exits_under_load() {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    let (tx, rx) = crossbeam_channel::bounded(64);
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);
    let (started_tx, started_rx) = crossbeam_channel::bounded(1);
    let handler = Arc::new(CountingHandler::with_first_signal(started_tx));
    let handler_trait: Arc<dyn FemtoHandlerTrait> = handler.clone();

    let running = Arc::new(AtomicBool::new(true));
    let producer_running = Arc::clone(&running);
    let producer_handler = handler_trait.clone();

    let producer = std::thread::spawn(move || {
        while producer_running.load(Ordering::Relaxed) {
            let record = QueuedRecord {
                record: FemtoLogRecord::new("core", FemtoLevel::Info, "load"),
                handlers: vec![producer_handler.clone()],
            };
            match tx.try_send(record) {
                Ok(()) => {}
                Err(crossbeam_channel::TrySendError::Full(_)) => std::thread::yield_now(),
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => break,
            }
        }
    });

    let worker = std::thread::spawn(move || {
        FemtoLogger::worker_thread_loop(rx, shutdown_rx);
        done_tx
            .send(())
            .expect("Failed to signal worker completion");
    });

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("Expected records before shutdown");
    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");
    running.store(false, Ordering::Relaxed);
    producer.join().expect("Producer thread panicked");

    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("Worker thread did not shutdown promptly");
    worker.join().expect("Worker thread panicked");
}

#[test]
fn clear_handlers_removes_all() {
    let logger = FemtoLogger::new("test".into());
    let h = Arc::new(CollectingHandler::new()) as Arc<dyn FemtoHandlerTrait>;
    logger.add_handler(h);
    assert_eq!(logger.handler_ptrs_for_test().len(), 1);
    logger.clear_handlers();
    assert!(logger.handler_ptrs_for_test().is_empty());
}

#[test]
fn multiple_filters_must_all_pass() {
    let logger = FemtoLogger::new("multi".into());
    let lvl = LevelFilterBuilder::new().with_max_level(FemtoLevel::Info);
    let name = NameFilterBuilder::new().with_prefix("multi");
    logger.add_filter(lvl.build().expect("level build should succeed"));
    logger.add_filter(name.build().expect("name build should succeed"));

    assert!(logger.log(FemtoLevel::Info, "ok").is_some());
    assert!(logger.log(FemtoLevel::Debug, "no").is_none());

    let other = FemtoLogger::new("other".into());
    other.add_filter(lvl.build().expect("level build should succeed"));
    other.add_filter(name.build().expect("name build should succeed"));
    assert!(other.log(FemtoLevel::Info, "no").is_none());
}

#[test]
fn removing_and_clearing_filters() {
    let logger = FemtoLogger::new("remove".into());
    let filt = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Info)
        .build()
        .expect("build should succeed");
    logger.add_filter(filt.clone());
    assert!(logger.log(FemtoLevel::Error, "msg").is_none());
    assert!(logger.remove_filter(&filt));
    assert!(logger.log(FemtoLevel::Error, "msg").is_some());

    let f1 = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Error)
        .build()
        .expect("build should succeed");
    let f2 = NameFilterBuilder::new()
        .with_prefix("other")
        .build()
        .expect("build should succeed");
    logger.add_filter(f1);
    logger.add_filter(f2);
    assert!(logger.log(FemtoLevel::Error, "msg").is_none());
    logger.clear_filters();
    assert!(logger.log(FemtoLevel::Error, "msg").is_some());
}

#[test]
fn removing_unknown_filter_returns_false() {
    let logger = FemtoLogger::new("unknown".into());
    let filt = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Info)
        .build()
        .expect("build should succeed");
    assert!(!logger.remove_filter(&filt));
    assert!(logger.log(FemtoLevel::Error, "err").is_some());
}

#[test]
fn drop_counter_increments_on_queue_overflow() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };

    struct BlockingHandler {
        started: Arc<Barrier>,
        release: Arc<Barrier>,
        waited: AtomicBool,
    }

    impl FemtoHandlerTrait for BlockingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            if !self.waited.swap(true, Ordering::SeqCst) {
                // Signal that we've started processing, then block until released
                self.started.wait();
                self.release.wait();
            }
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let handler = Arc::new(BlockingHandler {
        started: Arc::clone(&started),
        release: Arc::clone(&release),
        waited: AtomicBool::new(false),
    }) as Arc<dyn FemtoHandlerTrait>;
    let logger = FemtoLogger::new("drop".to_string());
    logger.add_handler(handler);
    logger.log(FemtoLevel::Info, "block");

    // Wait for worker to confirm it's processing (prevents race condition)
    started.wait();

    // Now safe to fill the queue - worker is blocked on release barrier
    for _ in 0..super::DEFAULT_CHANNEL_CAPACITY {
        logger.log(FemtoLevel::Info, "fill");
    }
    logger.log(FemtoLevel::Info, "overflow");
    assert_eq!(logger.get_dropped(), 1);

    // Release the worker to allow cleanup
    release.wait();
}

// Python integration tests are in a separate module to respect the 400-line limit.
#[cfg(feature = "python")]
#[path = "logger_tests_python.rs"]
mod logger_tests_python;
