//! Unit tests for FemtoLogger.

use super::*;
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder};
use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;

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
    fn handle(&self, record: FemtoLogRecord) {
        self.records.lock().push(record);
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
        record: FemtoLogRecord::new("core", "INFO", "msg"),
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
    assert_eq!(r1[0].message, "msg");
    assert_eq!(r2[0].message, "msg");
}

#[test]
fn drain_remaining_records_pulls_all() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let h = Arc::new(CollectingHandler::new());
    for i in 0..3 {
        tx.send(QueuedRecord {
            record: FemtoLogRecord::new("core", "INFO", &format!("{i}")),
            handlers: vec![h.clone() as Arc<dyn FemtoHandlerTrait>],
        })
        .expect("Failed to send test record");
    }
    drop(tx);

    FemtoLogger::drain_remaining_records(&rx);

    let msgs: Vec<String> = h.collected().into_iter().map(|r| r.message).collect();
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
        record: FemtoLogRecord::new("core", "INFO", "one"),
        handlers: vec![h.clone() as Arc<dyn FemtoHandlerTrait>],
    })
    .expect("Failed to send first test record");
    tx.send(QueuedRecord {
        record: FemtoLogRecord::new("core", "INFO", "two"),
        handlers: vec![h.clone() as Arc<dyn FemtoHandlerTrait>],
    })
    .expect("Failed to send second test record");
    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");
    thread.join().expect("Worker thread panicked");

    let msgs: Vec<String> = h.collected().into_iter().map(|r| r.message).collect();
    assert_eq!(msgs, vec!["one", "two"]);
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
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    };

    struct BlockingHandler {
        barrier: Arc<Barrier>,
        waited: AtomicBool,
    }

    impl FemtoHandlerTrait for BlockingHandler {
        fn handle(&self, _record: FemtoLogRecord) {
            if !self.waited.swap(true, Ordering::SeqCst) {
                self.barrier.wait();
            }
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let barrier = Arc::new(Barrier::new(2));
    let handler = Arc::new(BlockingHandler {
        barrier: Arc::clone(&barrier),
        waited: AtomicBool::new(false),
    }) as Arc<dyn FemtoHandlerTrait>;
    let logger = FemtoLogger::new("drop".to_string());
    logger.add_handler(handler);
    logger.log(FemtoLevel::Info, "block");
    for _ in 0..super::DEFAULT_CHANNEL_CAPACITY {
        logger.log(FemtoLevel::Info, "fill");
    }
    logger.log(FemtoLevel::Info, "overflow");
    assert_eq!(logger.get_dropped(), 1);
    barrier.wait();
}
