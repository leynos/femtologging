//! Focused unit tests for logger producer-path helpers.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rstest::{fixture, rstest};

use super::*;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::handler::FemtoHandlerTrait;
use crate::log_record::RecordMetadata;
use crate::test_utils::collecting_handler::CollectingHandler;

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

fn wait_for_record(handler: &CollectingHandler) -> Vec<FemtoLogRecord> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let collected = handler.collected();
        if !collected.is_empty() {
            return collected;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for queued record"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[fixture]
fn collecting_handler() -> Arc<CollectingHandler> {
    Arc::new(CollectingHandler::new())
}

#[rstest]
#[case::all_filters_accept(true, true, 1)]
#[case::second_filter_rejects(false, false, 0)]
fn apply_filters_merges_enrichment_and_short_circuits(
    collecting_handler: Arc<CollectingHandler>,
    #[case] second_accepts: bool,
    #[case] expected_result: bool,
    #[case] expected_third_calls: usize,
) {
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
    logger.add_handler(collecting_handler.clone() as Arc<dyn FemtoHandlerTrait>);

    let result = logger.log_with_metadata(FemtoLevel::Info, "hello", RecordMetadata::default());
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(third_calls.load(Ordering::SeqCst), expected_third_calls);

    if expected_result {
        assert_eq!(result.as_deref(), Some("producer [INFO] hello"));
        let collected = wait_for_record(&collecting_handler);
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

    assert_eq!(result.is_some(), expected_result);
}

#[rstest]
fn dispatch_to_handlers_enqueues_record_for_local_handlers(
    collecting_handler: Arc<CollectingHandler>,
) {
    let logger = FemtoLogger::new("producer".to_string());
    logger.add_handler(collecting_handler.clone() as Arc<dyn FemtoHandlerTrait>);

    logger.dispatch_to_handlers(FemtoLogRecord::new("producer", FemtoLevel::Info, "queued"));

    let collected = wait_for_record(&collecting_handler);
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].logger(), "producer");
    assert_eq!(collected[0].message(), "queued");
}
