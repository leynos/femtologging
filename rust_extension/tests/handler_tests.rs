//! Tests for the default `FemtoHandler` and for trait-object dispatch of
//! `FemtoHandlerTrait` methods onto user-supplied implementations.

use _femtologging_rs::{FemtoHandler, FemtoHandlerTrait, FemtoLogRecord, HandlerError};
#[cfg(feature = "python")]
use _femtologging_rs::{FemtoLevel, FemtoLogger};
#[cfg(feature = "python")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

/// Native handler double that records whether `flush` was dispatched.
#[derive(Default)]
struct DummyHandler {
    /// Shared flag set by the trait's flush implementation.
    flushed: Mutex<bool>,
}

impl FemtoHandlerTrait for DummyHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        // A test double must not mask a genuine failure by panicking on a
        // poisoned lock, so recover the inner value instead.
        let mut flag = self.flushed.lock().unwrap_or_else(PoisonError::into_inner);
        *flag = true;
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn default_handler_flush_returns_true() {
    let handler = FemtoHandler;
    assert!(handler.flush());
}

#[test]
fn legacy_flush_until_fails_closed_without_calling_flush() {
    let handler = DummyHandler::default();
    let trait_obj: &dyn FemtoHandlerTrait = &handler;
    assert!(!trait_obj.flush_until(std::time::Instant::now() + std::time::Duration::from_secs(1)));
    assert!(!*handler.flushed.lock().expect("flushed flag mutex poisoned"));
}

/// Handler double for checking default context-aware dispatch behavior.
#[cfg(feature = "python")]
struct DefaultDispatchHandler {
    /// Selects the Python-backed branch of the dispatch contract.
    python_backed: bool,
    /// Counts ordinary `handle` calls made by dispatch.
    handled_records: AtomicUsize,
}

#[cfg(feature = "python")]
impl FemtoHandlerTrait for DefaultDispatchHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.handled_records.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn is_python_backed(&self) -> bool {
        self.python_backed
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A Python-backed handler must not inherit context-discarding dispatch.
#[cfg(feature = "python")]
#[test]
fn python_backed_default_context_dispatch_fails_closed() {
    let handler = DefaultDispatchHandler {
        python_backed: true,
        handled_records: AtomicUsize::new(0),
    };

    let error = handler
        .handle_with_context(
            FemtoLogRecord::new("test", FemtoLevel::Info, "message"),
            None,
        )
        .expect_err("Python-backed handlers must implement contextual dispatch");

    assert_eq!(error, HandlerError::MissingContextDispatch);
    assert_eq!(handler.handled_records.load(Ordering::Relaxed), 0);
}

/// A Python-backed handler without contextual dispatch is rejected at registration.
#[cfg(feature = "python")]
#[test]
fn logger_rejects_python_handler_without_context_dispatch() {
    let logger = FemtoLogger::new("test".to_owned()).expect("test logger should be constructible");
    let handler: std::sync::Arc<dyn FemtoHandlerTrait> =
        std::sync::Arc::new(DefaultDispatchHandler {
            python_backed: true,
            handled_records: AtomicUsize::new(0),
        });

    assert_eq!(
        logger.add_handler(handler),
        Err(HandlerError::MissingContextDispatch)
    );
}

/// A native handler retains the trait's direct-dispatch default.
#[cfg(feature = "python")]
#[test]
fn native_default_context_dispatch_calls_handle() {
    let handler = DefaultDispatchHandler {
        python_backed: false,
        handled_records: AtomicUsize::new(0),
    };

    handler
        .handle_with_context(
            FemtoLogRecord::new("test", FemtoLevel::Info, "message"),
            None,
        )
        .expect("native handler should use direct dispatch");

    assert_eq!(handler.handled_records.load(Ordering::Relaxed), 1);
}
