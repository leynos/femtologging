//! Focused unit tests for logger producer-path helpers.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use super::logger_tests_helpers::{SignallingCollectingHandler, wait_for_record_signal};
use super::*;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::handler::FemtoHandlerTrait;
use crate::log_record::RecordMetadata;

struct TestFilter {
    accepted: bool,
    enrichment: BTreeMap<String, String>,
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
    logger.add_handler(Arc::new(signalling_handler) as Arc<dyn FemtoHandlerTrait>);

    let result = logger.log_with_metadata(FemtoLevel::Info, "hello", RecordMetadata::default());
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(third_calls.load(Ordering::SeqCst), expected_third_calls);

    if expected_result {
        assert_eq!(result.as_deref(), Some("producer [INFO] hello"));
        wait_for_record_signal(&record_rx);
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
    logger.add_handler(Arc::new(signalling_handler) as Arc<dyn FemtoHandlerTrait>);

    logger.log_with_metadata(
        FemtoLevel::Info,
        "message with conflicting enrichment",
        RecordMetadata::default(),
    );

    wait_for_record_signal(&record_rx);
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
    logger.add_handler(Arc::new(signalling_handler) as Arc<dyn FemtoHandlerTrait>);

    logger.dispatch_to_handlers(FemtoLogRecord::new("producer", FemtoLevel::Info, "queued"));

    wait_for_record_signal(&record_rx);
    let collected = collecting_handler.collected();
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].logger(), "producer");
    assert_eq!(collected[0].message(), "queued");
}
