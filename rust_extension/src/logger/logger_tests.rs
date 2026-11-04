//! Unit tests for FemtoLogger.

use super::*;
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use std::any::Any;
use std::sync::Arc;

use super::logger_tests_helpers::{CollectingHandler, collected_messages};

#[test]
fn clear_handlers_removes_all() {
    let logger = FemtoLogger::new("test".into()).expect("test logger should be constructible");
    let h = Arc::new(CollectingHandler::new()) as Arc<dyn FemtoHandlerTrait>;
    logger
        .add_handler(h)
        .expect("native test handler should register");
    assert_eq!(logger.handler_ptrs_for_test_native().len(), 1);
    logger.clear_handlers();
    assert!(logger.handler_ptrs_for_test_native().is_empty());
}

#[test]
fn multiple_filters_must_all_pass() {
    let logger = FemtoLogger::new("multi".into()).expect("test logger should be constructible");
    let lvl = LevelFilterBuilder::new().with_max_level(FemtoLevel::Info);
    let name = NameFilterBuilder::new().with_prefix("multi");
    logger.add_filter(lvl.build().expect("level build should succeed"));
    logger.add_filter(name.build().expect("name build should succeed"));

    assert!(logger.log(FemtoLevel::Info, "ok").is_some());
    assert!(logger.log(FemtoLevel::Debug, "no").is_none());

    let other = FemtoLogger::new("other".into()).expect("test logger should be constructible");
    other.add_filter(lvl.build().expect("level build should succeed"));
    other.add_filter(name.build().expect("name build should succeed"));
    assert!(other.log(FemtoLevel::Info, "no").is_none());
}

#[test]
fn removing_and_clearing_filters() {
    let logger = FemtoLogger::new("remove".into()).expect("test logger should be constructible");
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
    let logger = FemtoLogger::new("unknown".into()).expect("test logger should be constructible");
    let filt = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Info)
        .build()
        .expect("build should succeed");
    assert!(!logger.remove_filter(&filt));
    assert!(logger.log(FemtoLevel::Error, "err").is_some());
}

#[test]
fn logger_dispatches_through_its_shard() {
    let handler = Arc::new(CollectingHandler::new());
    let logger = FemtoLogger::new("sharded".into()).expect("test logger should be constructible");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    for message in ["one", "two", "three"] {
        logger.log(FemtoLevel::Info, message);
    }
    assert!(logger.flush_handlers());

    assert_eq!(collected_messages(&handler), vec!["one", "two", "three"]);
}

#[test]
fn flush_reports_drained_and_clears_the_backlog() {
    let handler = Arc::new(CollectingHandler::new());
    let logger = FemtoLogger::new("flush".into()).expect("test logger should be constructible");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    logger.log(FemtoLevel::Info, "queued");
    assert!(
        logger.flush_handlers(),
        "the shard should acknowledge a flush"
    );
    assert_eq!(
        collected_messages(&handler),
        vec!["queued"],
        "a flush must not return before the queued record is handled"
    );
}

#[test]
fn a_panicking_handler_does_not_silence_later_records() {
    /// Handler whose panic exercises per-handler worker recovery.
    struct PanickingHandler;

    impl FemtoHandlerTrait for PanickingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            panic!("handler boom");
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let handler = Arc::new(CollectingHandler::new());
    let logger = FemtoLogger::new("panic".into()).expect("test logger should be constructible");
    logger
        .add_handler(Arc::new(PanickingHandler) as Arc<dyn FemtoHandlerTrait>)
        .expect("native panicking test handler should register");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    logger.log(FemtoLevel::Info, "before panic");
    assert!(logger.flush_handlers());
    logger.log(FemtoLevel::Info, "after panic");
    assert!(logger.flush_handlers());

    assert_eq!(
        collected_messages(&handler),
        vec!["before panic", "after panic"],
        "a handler that panics must not stop the shard or the loggers on it"
    );
}

#[test]
fn a_handler_error_does_not_skip_later_handlers() {
    /// Handler that returns an error so later handlers can still be exercised.
    struct FailingHandler;

    impl FemtoHandlerTrait for FailingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            Err(HandlerError::Message("handler failed".to_owned()))
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let collecting_handler = Arc::new(CollectingHandler::new());
    let logger =
        FemtoLogger::new("handler-error".into()).expect("test logger should be constructible");
    logger
        .add_handler(Arc::new(FailingHandler) as Arc<dyn FemtoHandlerTrait>)
        .expect("native failing test handler should register");
    logger
        .add_handler(collecting_handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    logger.log(FemtoLevel::Info, "survives");
    assert!(logger.flush_handlers());

    assert_eq!(
        collected_messages(&collecting_handler),
        vec!["survives"],
        "a handler error must not deprive later handlers of the record"
    );
}

// Drop behaviour tests live in a separate module to respect
// the 400-line limit.
#[path = "logger_tests_drop.rs"]
mod logger_tests_drop;

// Drain-on-drop and pool-shape tests are in a separate module to respect
// the 400-line limit.
#[path = "logger_tests_shutdown.rs"]
mod logger_tests_shutdown;

// Python integration tests are in a separate module to respect the
// 400-line limit.
#[cfg(feature = "python")]
#[path = "logger_tests_python.rs"]
mod logger_tests_python;

// Convenience method tests (isEnabledFor, debug, info, etc.)
#[cfg(feature = "python")]
#[path = "logger_tests_convenience.rs"]
mod logger_tests_convenience;
