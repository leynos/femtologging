//! Focused unit tests for logger producer-path helpers.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};
use rstest::rstest;

#[cfg(feature = "python")]
use super::context_snapshot::ContextSnapshotProvider;
use super::logger_tests_helpers::{
    CountingHandler, SignallingCollectingHandler, wait_for_record_signal,
};
use super::*;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::RecordMetadata;

/// Filter fixture whose decision and enrichment can be inspected by tests.
struct TestFilter {
    /// Result returned for each record.
    accepted: bool,
    /// Fields merged into the record when accepted.
    enrichment: BTreeMap<String, String>,
    /// Shared count of evaluations across test references.
    calls: Arc<AtomicUsize>,
}

impl FemtoFilter for TestFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        FilterDecision {
            accepted: self.accepted,
            enrichment: self.enrichment.clone(),
        }
    }
}

fn enrichment_pair(key: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(key.to_owned(), value.to_owned())])
}

#[rstest]
#[case::all_filters_accept(true, true, 1)]
#[case::second_filter_rejects(false, false, 0)]
fn apply_filters_merges_enrichment_and_short_circuits(
    #[case] second_accepts: bool,
    #[case] expected_result: bool,
    #[case] expected_third_calls: usize,
) {
    let (collecting_handler, signalling_handler, record_rx) =
        SignallingCollectingHandler::with_signal();
    let logger = FemtoLogger::new("producer".to_string());
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let third_calls = Arc::new(AtomicUsize::new(0));

    logger.add_filter(Arc::new(TestFilter {
        accepted: true,
        enrichment: enrichment_pair("request_id", "req-123"),
        calls: Arc::clone(&first_calls),
    }));
    logger.add_filter(Arc::new(TestFilter {
        accepted: second_accepts,
        enrichment: enrichment_pair("user_id", "alice"),
        calls: Arc::clone(&second_calls),
    }));
    logger.add_filter(Arc::new(TestFilter {
        accepted: true,
        enrichment: enrichment_pair("ignored", "value"),
        calls: Arc::clone(&third_calls),
    }));
    logger
        .add_handler(Arc::new(signalling_handler) as Arc<dyn FemtoHandlerTrait>)
        .expect("native test handler should register");

    let result = logger.log_with_metadata(FemtoLevel::Info, "hello", RecordMetadata::default());
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(third_calls.load(Ordering::SeqCst), expected_third_calls);

    if expected_result {
        assert_eq!(result.as_deref(), Some("producer [INFO] hello"));
        wait_for_record_signal(&record_rx).expect("timed out waiting for queued record");
        let collected = collecting_handler.collected();
        assert_eq!(
            collected[0]
                .metadata()
                .key_values
                .get("request_id")
                .map(String::as_str),
            Some("req-123")
        );
        assert_eq!(
            collected[0]
                .metadata()
                .key_values
                .get("user_id")
                .map(String::as_str),
            Some("alice")
        );
        assert_eq!(
            collected[0]
                .metadata()
                .key_values
                .get("ignored")
                .map(String::as_str),
            Some("value")
        );
    } else {
        assert_eq!(result, None);
        assert!(collecting_handler.collected().is_empty());
    }
}

#[rstest]
fn apply_filters_conflicting_enrichment_prefers_later() {
    let (collecting_handler, signalling_handler, record_rx) =
        SignallingCollectingHandler::with_signal();
    let logger = FemtoLogger::new("producer".to_string());
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));

    logger.add_filter(Arc::new(TestFilter {
        accepted: true,
        enrichment: enrichment_pair("request_id", "first"),
        calls: Arc::clone(&first_calls),
    }));
    logger.add_filter(Arc::new(TestFilter {
        accepted: true,
        enrichment: enrichment_pair("request_id", "second"),
        calls: Arc::clone(&second_calls),
    }));
    logger
        .add_handler(Arc::new(signalling_handler) as Arc<dyn FemtoHandlerTrait>)
        .expect("native test handler should register");

    logger.log_with_metadata(
        FemtoLevel::Info,
        "message with conflicting enrichment",
        RecordMetadata::default(),
    );

    wait_for_record_signal(&record_rx).expect("timed out waiting for queued record");
    let collected = collecting_handler.collected();
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(collected.len(), 1);
    assert_eq!(
        collected[0]
            .metadata()
            .key_values
            .get("request_id")
            .map(String::as_str),
        Some("second")
    );
}

#[rstest]
fn dispatch_to_handlers_enqueues_record_for_local_handlers() {
    let (collecting_handler, signalling_handler, record_rx) =
        SignallingCollectingHandler::with_signal();
    let logger = FemtoLogger::new("producer".to_string());
    logger
        .add_handler(Arc::new(signalling_handler) as Arc<dyn FemtoHandlerTrait>)
        .expect("native test handler should register");

    logger.dispatch_to_handlers(FemtoLogRecord::new("producer", FemtoLevel::Info, "queued"));

    wait_for_record_signal(&record_rx).expect("timed out waiting for queued record");
    let collected = collecting_handler.collected();
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].logger(), "producer");
    assert_eq!(collected[0].message(), "queued");
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
        logger.add_handler_with_filters(
            Arc::new(ContextAwareTestHandler {
                handled_records: Arc::clone(&handled_records),
                records_with_context: Arc::clone(&records_with_context),
            }),
            vec![Arc::new(TestFilter {
                accepted: true,
                enrichment: BTreeMap::new(),
                calls: Arc::new(AtomicUsize::new(0)),
            })],
        )?;

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

struct BlockingHandler {
    started: Sender<()>,
    release: Receiver<()>,
}

impl FemtoHandlerTrait for BlockingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.started.send(()).map_err(|_| HandlerError::Closed)?;
        self.release.recv().map_err(|_| HandlerError::Closed)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[rstest]
fn rejected_handler_filters_do_not_enqueue_empty_jobs() {
    let logger = FemtoLogger::new("producer".to_string());
    let tx = logger
        .tx
        .as_ref()
        .expect("logger should have a queue")
        .clone();
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    logger
        .handlers
        .write()
        .push(HandlerAttachment::new(Arc::new(BlockingHandler {
            started: started_tx,
            release: release_rx,
        })));

    logger.dispatch_to_handlers(FemtoLogRecord::new(
        "producer",
        FemtoLevel::Info,
        "block-worker",
    ));
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("worker should be blocked inside the first handler");

    logger.handlers.write().clear();
    logger.handlers.write().push(HandlerAttachment {
        handler: Arc::new(CountingHandler::default()),
        filters: vec![Arc::new(TestFilter {
            accepted: false,
            enrichment: BTreeMap::new(),
            calls: Arc::new(AtomicUsize::new(0)),
        })],
    });
    logger.dispatch_to_handlers(FemtoLogRecord::new(
        "producer",
        FemtoLevel::Info,
        "filtered-out",
    ));

    let queued_jobs = tx.len();
    release_tx
        .send(())
        .expect("blocked handler should receive its release signal");

    assert_eq!(queued_jobs, 0, "rejected records must not occupy the queue");
}

#[path = "producer_snapshot_property_tests.rs"]
mod snapshot_property_tests;
