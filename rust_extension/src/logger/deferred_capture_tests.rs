//! Tests for deferred traceback capture around the filter pipeline.
//!
//! A recording filter reports *when* it ran relative to the capture hook,
//! which proves ordering without relying on payload shape; the sibling
//! capture-counter tests prove whether capture ran at all.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rstest::rstest;

use super::FemtoLogger;
use crate::filters::{
    FemtoFilter, FilterBuilderTrait, FilterContext, FilterDecision, LevelFilterBuilder,
};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;

/// One filter evaluation observed by [`RecordingFilter`].
struct Evaluation {
    /// Label of the filter that ran.
    filter: &'static str,
    /// Whether the capture hook had already run.
    after_capture: bool,
}

/// Filter that records its evaluation order and whether capture preceded it.
struct RecordingFilter {
    /// Label reported in the evaluation log.
    label: &'static str,
    /// Whether this filter claims to inspect traceback data.
    inspects_traceback: bool,
    /// Whether the filter accepts the record.
    accepts: bool,
    /// Shared capture flag set by the test's hook.
    captured: Arc<AtomicBool>,
    /// Shared log of evaluations in execution order.
    evaluations: Arc<Mutex<Vec<Evaluation>>>,
}

impl FemtoFilter for RecordingFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        self.evaluations
            .lock()
            .expect("evaluation log is not poisoned")
            .push(Evaluation {
                filter: self.label,
                after_capture: self.captured.load(Ordering::SeqCst),
            });
        FilterDecision::accept(self.accepts)
    }

    fn inspects_traceback(&self) -> bool {
        self.inspects_traceback
    }
}

/// Build a recording filter wired to the shared capture flag and log.
fn recording_filter(
    label: &'static str,
    inspects_traceback: bool,
    accepts: bool,
    captured: &Arc<AtomicBool>,
    evaluations: &Arc<Mutex<Vec<Evaluation>>>,
) -> Arc<dyn FemtoFilter> {
    Arc::new(RecordingFilter {
        label,
        inspects_traceback,
        accepts,
        captured: Arc::clone(captured),
        evaluations: Arc::clone(evaluations),
    })
}

/// Return the labels of every recorded evaluation, in order.
fn evaluation_order(evaluations: &Arc<Mutex<Vec<Evaluation>>>) -> Vec<(&'static str, bool)> {
    evaluations
        .lock()
        .expect("evaluation log is not poisoned")
        .iter()
        .map(|entry| (entry.filter, entry.after_capture))
        .collect()
}

/// Build a logger whose level admits `FemtoLevel::Info` records.
fn info_logger(name: &str) -> FemtoLogger {
    let logger = FemtoLogger::new(name.to_owned()).expect("test logger should be constructible");
    logger.set_level(FemtoLevel::Trace);
    logger
}

#[test]
fn native_prefix_rejection_skips_capture_and_later_filters() {
    let logger = info_logger("deferred-reject");
    let captured = Arc::new(AtomicBool::new(false));
    let evaluations = Arc::new(Mutex::new(Vec::new()));
    logger.add_filter(recording_filter(
        "rejecting",
        false,
        false,
        &captured,
        &evaluations,
    ));
    logger.add_filter(recording_filter(
        "later",
        true,
        true,
        &captured,
        &evaluations,
    ));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-reject", FemtoLevel::Info, "ignored"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            captured.store(true, Ordering::SeqCst);
        },
    );

    assert!(result.is_none(), "a rejecting prefix must drop the record");
    assert_eq!(
        captures.load(Ordering::SeqCst),
        0,
        "capture must not run for a record the prefix rejected"
    );
    assert_eq!(
        evaluation_order(&evaluations),
        vec![("rejecting", false)],
        "filters after the rejection must not run"
    );
}

#[test]
fn capture_runs_between_prefix_and_remaining_filters() {
    let logger = info_logger("deferred-accept");
    let captured = Arc::new(AtomicBool::new(false));
    let evaluations = Arc::new(Mutex::new(Vec::new()));
    logger.add_filter(recording_filter(
        "prefix",
        false,
        true,
        &captured,
        &evaluations,
    ));
    logger.add_filter(recording_filter(
        "inspecting",
        true,
        true,
        &captured,
        &evaluations,
    ));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-accept", FemtoLevel::Info, "kept"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            captured.store(true, Ordering::SeqCst);
        },
    );

    assert!(
        result.is_some(),
        "an accepting pipeline should format the record"
    );
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert_eq!(
        evaluation_order(&evaluations),
        vec![("prefix", false), ("inspecting", true)],
        "capture must sit between the non-inspecting prefix and the rest"
    );
}

#[test]
fn deferral_does_not_reorder_filters() {
    let logger = info_logger("deferred-order");
    let captured = Arc::new(AtomicBool::new(false));
    let evaluations = Arc::new(Mutex::new(Vec::new()));
    logger.add_filter(recording_filter(
        "first",
        false,
        true,
        &captured,
        &evaluations,
    ));
    logger.add_filter(recording_filter(
        "inspecting",
        true,
        true,
        &captured,
        &evaluations,
    ));
    logger.add_filter(recording_filter(
        "third",
        false,
        true,
        &captured,
        &evaluations,
    ));

    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-order", FemtoLevel::Info, "kept"),
        |_| captured.store(true, Ordering::SeqCst),
    );

    assert!(result.is_some());
    assert_eq!(
        evaluation_order(&evaluations),
        vec![("first", false), ("inspecting", true), ("third", true)],
        "a non-inspecting filter after a callback-first filter must stay put"
    );
}

#[test]
fn rejection_after_capture_prevents_formatting() {
    let logger = info_logger("deferred-late-reject");
    let captured = Arc::new(AtomicBool::new(false));
    let evaluations = Arc::new(Mutex::new(Vec::new()));
    logger.add_filter(recording_filter(
        "prefix",
        false,
        true,
        &captured,
        &evaluations,
    ));
    logger.add_filter(recording_filter(
        "rejecting",
        true,
        false,
        &captured,
        &evaluations,
    ));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-late-reject", FemtoLevel::Info, "ignored"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            captured.store(true, Ordering::SeqCst);
        },
    );

    assert!(result.is_none(), "the later rejection must drop the record");
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert_eq!(
        evaluation_order(&evaluations),
        vec![("prefix", false), ("rejecting", true)]
    );
}

#[test]
fn below_threshold_skips_capture_and_filters() {
    let logger = FemtoLogger::new("deferred-threshold".to_owned())
        .expect("test logger should be constructible");
    logger.set_level(FemtoLevel::Error);
    let captured = Arc::new(AtomicBool::new(false));
    let evaluations = Arc::new(Mutex::new(Vec::new()));
    logger.add_filter(recording_filter(
        "any",
        false,
        true,
        &captured,
        &evaluations,
    ));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-threshold", FemtoLevel::Info, "ignored"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            captured.store(true, Ordering::SeqCst);
        },
    );

    assert!(result.is_none());
    assert_eq!(captures.load(Ordering::SeqCst), 0);
    assert!(
        evaluation_order(&evaluations).is_empty(),
        "filters must not run below the logger threshold"
    );
}

#[rstest]
#[case::level_only_filter(false)]
#[case::name_only_filter(true)]
fn native_filters_declare_they_ignore_traceback(#[case] name_filter: bool) {
    use crate::filters::NameFilterBuilder;

    let filter = if name_filter {
        NameFilterBuilder::new()
            .with_prefix("app")
            .build()
            .expect("name filter should build")
    } else {
        LevelFilterBuilder::new()
            .with_max_level(FemtoLevel::Info)
            .build()
            .expect("level filter should build")
    };

    assert!(
        !filter.inspects_traceback(),
        "native metadata filters must opt out of traceback inspection"
    );
}
