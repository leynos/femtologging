//! Unit tests for FemtoLogger.

use super::*;
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder};
use crate::handler::{FemtoHandlerTrait, HandlerError};
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
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.records.lock().push(record);
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
    assert_eq!(r1[0].message, "msg");
    assert_eq!(r2[0].message, "msg");
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
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };

    struct BlockingHandler {
        barrier: Arc<Barrier>,
        waited: AtomicBool,
    }

    impl FemtoHandlerTrait for BlockingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            if !self.waited.swap(true, Ordering::SeqCst) {
                self.barrier.wait();
            }
            Ok(())
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

// --------------------------------
// Tests for should_capture_exc_info
// --------------------------------

#[cfg(feature = "python")]
mod should_capture_exc_info_tests {
    use super::super::should_capture_exc_info;
    use pyo3::prelude::*;
    use pyo3::types::{PyBool, PyTuple};

    #[test]
    fn returns_true_for_true() {
        Python::with_gil(|py| {
            let true_val = PyBool::new(py, true);
            let result = should_capture_exc_info(true_val.as_any())
                .expect("should_capture_exc_info should not fail with True");
            assert!(result, "True should trigger capture");
        });
    }

    #[test]
    fn returns_false_for_false() {
        Python::with_gil(|py| {
            let false_val = PyBool::new(py, false);
            let result = should_capture_exc_info(false_val.as_any())
                .expect("should_capture_exc_info should not fail with False");
            assert!(!result, "False should not trigger capture");
        });
    }

    #[test]
    fn returns_false_for_none() {
        Python::with_gil(|py| {
            let none = py.None();
            let result = should_capture_exc_info(none.bind(py))
                .expect("should_capture_exc_info should not fail with None");
            assert!(!result, "None should not trigger capture");
        });
    }

    #[test]
    fn returns_true_for_exception_instance() {
        Python::with_gil(|py| {
            let exc = py
                .import("builtins")
                .expect("builtins module should exist")
                .getattr("ValueError")
                .expect("ValueError should exist")
                .call1(("test error",))
                .expect("ValueError constructor should succeed");

            let result = should_capture_exc_info(&exc)
                .expect("should_capture_exc_info should not fail with exception instance");
            assert!(result, "Exception instance should trigger capture");
        });
    }

    #[test]
    fn returns_true_for_3_tuple() {
        Python::with_gil(|py| {
            let exc_type = py
                .import("builtins")
                .expect("builtins module should exist")
                .getattr("KeyError")
                .expect("KeyError should exist");
            let exc_value = exc_type
                .call1(("key",))
                .expect("KeyError constructor should succeed");
            let exc_tb = py.None();

            let tuple = PyTuple::new(
                py,
                &[exc_type.as_any(), exc_value.as_any(), exc_tb.bind(py)],
            )
            .expect("tuple creation should succeed");

            let result = should_capture_exc_info(tuple.as_any())
                .expect("should_capture_exc_info should not fail with tuple");
            assert!(result, "3-tuple should trigger capture");
        });
    }

    #[test]
    fn returns_true_for_integer() {
        // Integer is truthy and not a bool, so it should trigger capture
        // (even though capture_exception will later fail)
        Python::with_gil(|py| {
            let code = c"42";
            let int_val = py
                .eval(code, None, None)
                .expect("eval of integer should succeed");
            let result = should_capture_exc_info(&int_val)
                .expect("should_capture_exc_info should not fail with integer");
            assert!(result, "Non-None non-False values should trigger capture");
        });
    }
}

// --------------------------------
// Tests for py_log
// --------------------------------

#[cfg(feature = "python")]
mod py_log_tests {
    use super::super::FemtoLogger;
    use super::*;
    use pyo3::Python;
    use pyo3::types::PyBool;

    #[test]
    fn py_log_basic_message() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            let result = logger
                .py_log(py, FemtoLevel::Info, "hello", None, None)
                .expect("py_log should not fail");
            assert_eq!(result, Some("test [INFO] hello".to_string()));
        });
    }

    #[test]
    fn py_log_filtered_by_level() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            logger.set_level(FemtoLevel::Error);
            let result = logger
                .py_log(py, FemtoLevel::Info, "ignored", None, None)
                .expect("py_log should not fail");
            assert!(
                result.is_none(),
                "Message below level threshold should be filtered"
            );
        });
    }

    #[test]
    fn py_log_with_exc_info_false() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            let false_val = PyBool::new(py, false);
            let result = logger
                .py_log(
                    py,
                    FemtoLevel::Error,
                    "no traceback",
                    Some(false_val.as_any()),
                    None,
                )
                .expect("py_log should not fail with exc_info=False");
            assert_eq!(result, Some("test [ERROR] no traceback".to_string()));
        });
    }

    #[test]
    fn py_log_with_exc_info_none() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            let none = py.None();
            let result = logger
                .py_log(
                    py,
                    FemtoLevel::Error,
                    "no traceback",
                    Some(none.bind(py)),
                    None,
                )
                .expect("py_log should not fail with exc_info=None");
            assert_eq!(result, Some("test [ERROR] no traceback".to_string()));
        });
    }

    #[test]
    fn py_log_with_stack_info_false() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            let result = logger
                .py_log(py, FemtoLevel::Info, "no stack", None, Some(false))
                .expect("py_log should not fail with stack_info=false");
            assert_eq!(result, Some("test [INFO] no stack".to_string()));
        });
    }

    #[test]
    fn py_log_with_stack_info_true() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            let result = logger
                .py_log(py, FemtoLevel::Info, "with stack", None, Some(true))
                .expect("py_log should not fail with stack_info=true");

            assert!(result.is_some(), "Should produce output");
            let output = result.expect("output should be Some");
            assert!(
                output.contains("test [INFO] with stack"),
                "Should contain base message"
            );
            assert!(output.contains("Stack"), "Should contain stack trace");
        });
    }

    #[test]
    fn py_log_with_exception_instance() {
        Python::with_gil(|py| {
            let logger = FemtoLogger::new("test".to_string());
            let exc = py
                .import("builtins")
                .expect("builtins module should exist")
                .getattr("ValueError")
                .expect("ValueError should exist")
                .call1(("test error",))
                .expect("ValueError constructor should succeed");

            let result = logger
                .py_log(py, FemtoLevel::Error, "caught", Some(&exc), None)
                .expect("py_log should not fail with exception instance");

            assert!(result.is_some(), "Should produce output");
            let output = result.expect("output should be Some");
            assert!(
                output.contains("test [ERROR] caught"),
                "Should contain base message"
            );
            assert!(
                output.contains("ValueError"),
                "Should contain exception type"
            );
            assert!(
                output.contains("test error"),
                "Should contain exception message"
            );
        });
    }
}
