//! Regression coverage for producer-side Python context snapshots.
//!
//! Gated on the `python` feature, these tests inject
//! `TestContextSnapshotProvider` (a `ContextSnapshotProvider`) with a
//! `ContextAwareTestHandler`. `ContextCaptureFixture` bundles logger and
//! counters so `injected_context_snapshot_provider_supplies_queued_context`
//! proves captured context reaches the queue, while
//! `context_capture_failures_have_a_separate_counter_from_queue_drops` keeps
//! capture failures out of `get_dropped` and
//! `destruction_reports_context_capture_drops_held_back_by_the_interval` pins
//! their report at destruction.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::context_snapshot::ContextSnapshotProvider;
use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::rate_limited_warner::{Clock, DEFAULT_WARN_INTERVAL, RateLimitedWarner};

/// A clock that never advances, so a warning cannot become due by elapsed time.
#[cfg(feature = "python")]
struct FixedClock;

#[cfg(feature = "python")]
impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        0
    }
}

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

    fn flush_until(&self, _context: &crate::handler::FlushContext) -> bool {
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
        // The interval decides whether a second failed capture is suppressed,
        // so the clock is fixed rather than left to the scheduler: a pause
        // longer than five seconds would let the second failure report itself.
        let logger =
            FemtoLogger::with_context_snapshot_provider("producer".to_owned(), None, provider)
                .with_context_capture_warner_for_test(RateLimitedWarner::with_clock(
                    DEFAULT_WARN_INTERVAL,
                    Arc::new(FixedClock),
                ));
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
///
/// `#[serial]` because the first failure warns through the process-wide log
/// capture that the neighbouring flush test asserts on.
#[cfg(feature = "python")]
#[test]
#[serial_test::serial]
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

/// Destruction reports context-capture drops the warning interval held back.
///
/// Scenario: two captures fail inside one interval, so the first warns and the
/// second is suppressed, the logger is flushed, then it is destroyed.
/// Invariant: the deadline-bound flush reports nothing and leaves the count
/// pending, and destruction - which has no caller deadline - reports it once. A
/// blocking `log` sink must not be able to push `flush_handlers()` past its
/// timeout, so the count waits for the path that can afford the write.
#[cfg(feature = "python")]
#[test]
#[serial_test::serial]
fn destruction_reports_context_capture_drops_held_back_by_the_interval() {
    use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};

    install_test_logger();
    let fixture =
        ContextCaptureFixture::new(true).expect("context-aware test handler should register");

    fixture.logger.log(FemtoLevel::Info, "first failed capture");
    fixture
        .logger
        .log(FemtoLevel::Info, "second failed capture");
    assert_eq!(fixture.captures.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.logger.get_context_capture_failures(), 2);
    let suppression_warnings = take_logged_messages()
        .into_iter()
        .filter(|warning| {
            warning.message == "FemtoLogger: dropped 1 records after context capture failed"
        })
        .count();
    assert_eq!(
        suppression_warnings, 1,
        "only the first failed capture should warn inside the interval"
    );
    assert_eq!(
        fixture.logger.context_capture_warner.pending_drops(),
        1,
        "the suppressed failure must stay pending rather than be discarded",
    );

    assert!(
        fixture.logger.flush_handlers(),
        "logger worker did not flush"
    );
    assert_eq!(
        fixture.logger.context_capture_warner.pending_drops(),
        1,
        "the deadline-bound flush must leave the count pending"
    );
    let flush_warnings = take_logged_messages()
        .into_iter()
        .filter(|warning| {
            warning.message == "FemtoLogger: dropped 1 records after context capture failed"
        })
        .count();
    assert_eq!(
        flush_warnings, 0,
        "the flush must not report the count synchronously"
    );

    drop(fixture.logger);

    let destruction_warnings = take_logged_messages()
        .into_iter()
        .filter(|warning| {
            warning.message == "FemtoLogger: dropped 1 records after context capture failed"
        })
        .count();
    assert_eq!(
        destruction_warnings, 1,
        "destruction should report the suppressed failure exactly once"
    );
}
