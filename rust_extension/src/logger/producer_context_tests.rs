//! Regression coverage for producer-side Python context snapshots.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::context_snapshot::ContextSnapshotProvider;
use super::*;
use crate::handler::FemtoHandlerTrait;

/// Injectable context source that counts captures and can return a Python error.
#[cfg(feature = "python")]
struct TestContextSnapshotProvider {
    /// Shared count of snapshot requests.
    captures: Arc<AtomicUsize>,
    /// Makes capture return a deterministic Python exception.
    fail: bool,
}

#[cfg(feature = "python")]
impl ContextSnapshotProvider for TestContextSnapshotProvider {
    fn capture(&self) -> PyResult<Py<PyAny>> {
        self.captures.fetch_add(1, Ordering::Relaxed);
        if self.fail {
            Err(pyo3::exceptions::PyRuntimeError::new_err(
                "context provider failed",
            ))
        } else {
            Python::attach(|py| Ok(py.None()))
        }
    }
}

/// Python-backed handler fixture that observes captured context delivery.
#[cfg(feature = "python")]
struct ContextAwareTestHandler {
    /// Shared count of records delivered to this handler.
    handled_records: Arc<AtomicUsize>,
    /// Shared count of records delivered with a context snapshot.
    records_with_context: Arc<AtomicUsize>,
}

#[cfg(feature = "python")]
impl FemtoHandlerTrait for ContextAwareTestHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), crate::handler::HandlerError> {
        self.handled_records.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn flush_until(&self, _deadline: std::time::Instant) -> bool {
        true
    }

    fn is_python_backed(&self) -> bool {
        true
    }

    fn provides_context_dispatch(&self) -> bool {
        true
    }

    fn handle_with_context(
        &self,
        _record: FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), crate::handler::HandlerError> {
        self.handled_records.fetch_add(1, Ordering::Relaxed);
        if context.is_some() {
            self.records_with_context.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Bundle of logger, provider and counters for context-dispatch tests.
#[cfg(feature = "python")]
struct ContextCaptureFixture {
    /// Logger configured with the injected context provider and handler.
    logger: FemtoLogger,
    /// Count of context snapshots captured before dispatch.
    captures: Arc<AtomicUsize>,
    /// Count of records received by the Python-backed handler.
    handled_records: Arc<AtomicUsize>,
    /// Count of received records carrying captured context.
    records_with_context: Arc<AtomicUsize>,
}

#[cfg(feature = "python")]
impl ContextCaptureFixture {
    fn new(fail: bool) -> Result<Self, crate::handler::HandlerError> {
        let captures = Arc::new(AtomicUsize::new(0));
        let handled_records = Arc::new(AtomicUsize::new(0));
        let records_with_context = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn ContextSnapshotProvider> = Arc::new(TestContextSnapshotProvider {
            captures: Arc::clone(&captures),
            fail,
        });
        let logger =
            FemtoLogger::with_context_snapshot_provider("producer".to_owned(), None, provider);
        logger.add_handler(Arc::new(ContextAwareTestHandler {
            handled_records: Arc::clone(&handled_records),
            records_with_context: Arc::clone(&records_with_context),
        }))?;

        Ok(Self {
            logger,
            captures,
            handled_records,
            records_with_context,
        })
    }
}

/// The injected provider's snapshot reaches the queued Python handler.
#[cfg(feature = "python")]
#[test]
fn injected_context_snapshot_provider_supplies_queued_context() {
    let fixture =
        ContextCaptureFixture::new(false).expect("context-aware test handler should register");

    fixture.logger.log(FemtoLevel::Info, "message");
    assert!(
        fixture.logger.flush_handlers(),
        "logger worker did not flush"
    );

    assert_eq!(fixture.captures.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.handled_records.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.records_with_context.load(Ordering::Relaxed), 1);
}

/// Context capture failures do not increment the queue-drop counter.
#[cfg(feature = "python")]
#[test]
fn context_capture_failures_have_a_separate_counter_from_queue_drops() {
    let fixture =
        ContextCaptureFixture::new(true).expect("context-aware test handler should register");

    fixture.logger.log(FemtoLevel::Info, "message");

    assert_eq!(fixture.captures.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.logger.get_context_capture_failures(), 1);
    assert_eq!(fixture.logger.get_dropped(), 0);
    assert_eq!(fixture.handled_records.load(Ordering::Relaxed), 0);
    assert!(
        fixture.logger.flush_handlers(),
        "logger worker did not flush"
    );
    assert_eq!(fixture.handled_records.load(Ordering::Relaxed), 0);
}
