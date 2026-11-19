//! Tests for deferred traceback capture around the filter pipeline.
//!
//! A recording filter reports *when* it ran relative to the capture hook,
//! which proves ordering without relying on payload shape; the sibling
//! capture-counter tests prove whether capture ran at all.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rstest::{fixture, rstest};

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

/// Capture flag and evaluation log shared by one test's filters.
///
/// A test that wires several recording filters needs every one of them to share
/// the same two handles, so a fixture owns them rather than each test spelling
/// out the same allocation and cloning.
struct Observations {
    /// Set by the test's capture hook.
    captured: Arc<AtomicBool>,
    /// Evaluations in execution order.
    evaluations: Arc<Mutex<Vec<Evaluation>>>,
}

impl Observations {
    /// Build a recording filter wired to these observations.
    fn filter(
        &self,
        label: &'static str,
        inspects_traceback: bool,
        accepts: bool,
    ) -> Arc<dyn FemtoFilter> {
        Arc::new(RecordingFilter {
            label,
            inspects_traceback,
            accepts,
            captured: Arc::clone(&self.captured),
            evaluations: Arc::clone(&self.evaluations),
        })
    }

    /// Return the labels of every recorded evaluation, in order.
    fn order(&self) -> Vec<(&'static str, bool)> {
        self.evaluations
            .lock()
            .expect("evaluation log is not poisoned")
            .iter()
            .map(|entry| (entry.filter, entry.after_capture))
            .collect()
    }

    /// Report that the capture hook has run.
    fn mark_captured(&self) {
        self.captured.store(true, Ordering::SeqCst);
    }
}

/// Provide a fresh pair of shared handles to each test.
#[fixture]
fn observations() -> Observations {
    Observations {
        captured: Arc::new(AtomicBool::new(false)),
        evaluations: Arc::new(Mutex::new(Vec::new())),
    }
}

/// Build a logger whose level admits `FemtoLevel::Info` records.
fn info_logger(name: &str) -> FemtoLogger {
    let logger = FemtoLogger::new(name.to_owned()).expect("test logger should be constructible");
    logger.set_level(FemtoLevel::Trace);
    logger
}

#[rstest]
fn native_prefix_rejection_skips_capture_and_later_filters(observations: Observations) {
    let logger = info_logger("deferred-reject");
    logger.add_filter(observations.filter("rejecting", false, false));
    logger.add_filter(observations.filter("later", true, true));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-reject", FemtoLevel::Info, "ignored"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            observations.mark_captured();
        },
    );

    assert!(result.is_none(), "a rejecting prefix must drop the record");
    assert_eq!(
        captures.load(Ordering::SeqCst),
        0,
        "capture must not run for a record the prefix rejected"
    );
    assert_eq!(
        observations.order(),
        vec![("rejecting", false)],
        "filters after the rejection must not run"
    );
}

#[rstest]
fn capture_runs_between_prefix_and_remaining_filters(observations: Observations) {
    let logger = info_logger("deferred-accept");
    logger.add_filter(observations.filter("prefix", false, true));
    logger.add_filter(observations.filter("inspecting", true, true));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-accept", FemtoLevel::Info, "kept"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            observations.mark_captured();
        },
    );

    assert!(
        result.is_some(),
        "an accepting pipeline should format the record"
    );
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert_eq!(
        observations.order(),
        vec![("prefix", false), ("inspecting", true)],
        "capture must sit between the non-inspecting prefix and the rest"
    );
}

#[rstest]
fn deferral_does_not_reorder_filters(observations: Observations) {
    let logger = info_logger("deferred-order");
    logger.add_filter(observations.filter("first", false, true));
    logger.add_filter(observations.filter("inspecting", true, true));
    logger.add_filter(observations.filter("third", false, true));

    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-order", FemtoLevel::Info, "kept"),
        |_| observations.mark_captured(),
    );

    assert!(result.is_some());
    assert_eq!(
        observations.order(),
        vec![("first", false), ("inspecting", true), ("third", true)],
        "a non-inspecting filter after a callback-first filter must stay put"
    );
}

#[rstest]
fn rejection_after_capture_prevents_formatting(observations: Observations) {
    let logger = info_logger("deferred-late-reject");
    logger.add_filter(observations.filter("prefix", false, true));
    logger.add_filter(observations.filter("rejecting", true, false));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-late-reject", FemtoLevel::Info, "ignored"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            observations.mark_captured();
        },
    );

    assert!(result.is_none(), "the later rejection must drop the record");
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert_eq!(
        observations.order(),
        vec![("prefix", false), ("rejecting", true)]
    );
}

#[rstest]
fn below_threshold_skips_capture_and_filters(observations: Observations) {
    let logger = FemtoLogger::new("deferred-threshold".to_owned())
        .expect("test logger should be constructible");
    logger.set_level(FemtoLevel::Error);
    logger.add_filter(observations.filter("any", false, true));

    let captures = AtomicUsize::new(0);
    let result = logger.log_record_with_capture(
        FemtoLogRecord::new("deferred-threshold", FemtoLevel::Info, "ignored"),
        |_| {
            captures.fetch_add(1, Ordering::SeqCst);
            observations.mark_captured();
        },
    );

    assert!(result.is_none());
    assert_eq!(captures.load(Ordering::SeqCst), 0);
    assert!(
        observations.order().is_empty(),
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
