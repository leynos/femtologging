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

    let before = flush_metrics_snapshot();
    logger.log(FemtoLevel::Info, "queued");
    assert!(
        logger.flush_handlers(),
        "the shard should acknowledge a flush"
    );
    let after = flush_metrics_snapshot();

    assert!(
        after.outcomes[FlushOutcome::Success.index()]
            > before.outcomes[FlushOutcome::Success.index()],
        "a successful flush must increment the process-wide success counter",
    );
    assert!(
        after.duration_count > before.duration_count,
        "a successful flush must record a duration",
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

/// Handler whose flush result and call count a test controls.
///
/// Kept beside the best-effort test rather than in the shared helper module:
/// it exists only to prove that a failing handler does not stop the flush of
/// the handlers after it.
struct FlushOutcomeHandler {
    /// Result this handler reports from every flush.
    succeeds: bool,
    /// Number of flush calls observed so far.
    calls: std::sync::atomic::AtomicUsize,
}

impl FlushOutcomeHandler {
    /// Create a handler that always reports *succeeds* from `flush`.
    fn new(succeeds: bool) -> Self {
        Self {
            succeeds,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Return how many times `flush` has been called.
    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl FemtoHandlerTrait for FlushOutcomeHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.succeeds
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn a_failing_handler_flush_does_not_skip_later_handlers() {
    let failing = Arc::new(FlushOutcomeHandler::new(false));
    let succeeding = Arc::new(FlushOutcomeHandler::new(true));
    let logger =
        FemtoLogger::new("flush-best-effort".into()).expect("test logger should be constructible");
    logger
        .add_handler(failing.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native failing flush handler should register");
    logger
        .add_handler(succeeding.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native succeeding flush handler should register");

    let before = flush_metrics_snapshot();
    assert!(
        !logger.flush_handlers(),
        "a handler that fails to flush must make the logger's flush report failure"
    );
    let after = flush_metrics_snapshot();

    assert!(
        after.outcomes[FlushOutcome::HandlerFailure.index()]
            > before.outcomes[FlushOutcome::HandlerFailure.index()],
        "a failed handler flush must increment the handler-failure counter",
    );
    assert_eq!(failing.calls(), 1, "the failing handler must be flushed");
    assert_eq!(
        succeeding.calls(),
        1,
        "a flush must continue past a handler that reported failure"
    );
}

/// Handler that attaches a second handler to its own logger during a flush.
///
/// A flush snapshots the handler list and releases the guard before calling
/// any handler. Calling `add_handler` from inside `flush` is the cheapest way
/// to prove that: a flush that kept the read guard would deadlock here on the
/// write guard the attachment takes.
struct AttachingFlushHandler {
    /// Logger to attach to, set once the logger exists.
    logger: std::sync::Mutex<Option<Arc<FemtoLogger>>>,
    /// Handler the first flush attaches.
    attached: Arc<CollectingHandler>,
    /// Whether the attachment has already happened.
    attached_once: std::sync::atomic::AtomicBool,
}

impl AttachingFlushHandler {
    /// Create a handler that will attach *attached* on its first flush.
    fn new(attached: Arc<CollectingHandler>) -> Self {
        Self {
            logger: std::sync::Mutex::new(None),
            attached,
            attached_once: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl FemtoHandlerTrait for AttachingFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        if !self
            .attached_once
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let logger = self
                .logger
                .lock()
                .expect("the attach-once logger lock is never poisoned")
                .clone();
            if let Some(logger) = logger {
                logger
                    .add_handler(Arc::clone(&self.attached) as Arc<dyn FemtoHandlerTrait>)
                    .expect("a native handler may be attached during a flush");
            }
        }
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn a_handler_flush_may_attach_another_handler() {
    let logger = Arc::new(
        FemtoLogger::new("flush-attach".into()).expect("test logger should be constructible"),
    );
    let attached = Arc::new(CollectingHandler::new());
    let attaching = Arc::new(AttachingFlushHandler::new(Arc::clone(&attached)));
    logger
        .add_handler(Arc::clone(&attaching) as Arc<dyn FemtoHandlerTrait>)
        .expect("native attaching handler should register");
    *attaching
        .logger
        .lock()
        .expect("the attach-once logger lock is never poisoned") = Some(Arc::clone(&logger));

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let flush_logger = Arc::clone(&logger);
    std::thread::spawn(move || {
        let _ = done_tx.send(flush_logger.flush_handlers());
    });
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a handler that attaches during a flush must not deadlock"),
        "attaching a handler during a flush must not fail the flush",
    );

    logger.log(FemtoLevel::Info, "after attachment");
    assert!(logger.flush_handlers(), "the logger should still flush");
    assert_eq!(
        collected_messages(&attached),
        vec!["after attachment"],
        "a handler attached during a flush must receive later records",
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
