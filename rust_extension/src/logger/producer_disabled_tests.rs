//! Producer-path tests for configuration-disabled loggers.
//!
//! A logger disabled by `disable_existing_loggers` must reject records before
//! level filtering, filters, formatting and dispatch. These tests instrument
//! the filter and the logger formatter so a rejection that still ran either
//! stage would fail rather than merely return `None`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::formatter::{FemtoFormatter, SharedFormatter};

/// Filter that counts how often the producer path reached filtering.
struct CountingFilter(Arc<AtomicUsize>);

impl FemtoFilter for CountingFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        self.0.fetch_add(1, Ordering::SeqCst);
        FilterDecision {
            accepted: true,
            callback_failed: false,
            enrichment: BTreeMap::new(),
        }
    }
}

/// Formatter that counts how often the producer path reached formatting.
struct CountingFormatter(Arc<AtomicUsize>);

impl FemtoFormatter for CountingFormatter {
    fn format(&self, _record: &FemtoLogRecord) -> String {
        self.0.fetch_add(1, Ordering::SeqCst);
        "formatted".to_owned()
    }
}

/// Build a logger whose filter and formatter count their invocations.
fn counting_logger() -> (FemtoLogger, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let filter_calls = Arc::new(AtomicUsize::new(0));
    let format_calls = Arc::new(AtomicUsize::new(0));
    let logger = FemtoLogger::with_formatter_for_test(
        "counting".to_string(),
        SharedFormatter::new(CountingFormatter(Arc::clone(&format_calls))),
    );
    logger.add_filter(Arc::new(CountingFilter(Arc::clone(&filter_calls))));
    (logger, filter_calls, format_calls)
}

/// A disabled logger rejects before filters, formatting and dispatch.
#[test]
fn disabled_logger_skips_filters_and_formatting() {
    let (logger, filter_calls, format_calls) = counting_logger();
    logger.set_disabled(true);

    assert_eq!(
        logger.log(FemtoLevel::Error, "suppressed"),
        None,
        "a disabled logger must reject the record outright",
    );
    assert_eq!(
        filter_calls.load(Ordering::SeqCst),
        0,
        "a disabled logger must not run its filters",
    );
    assert_eq!(
        format_calls.load(Ordering::SeqCst),
        0,
        "a disabled logger must not run its formatter",
    );
    assert_eq!(
        logger.get_disabled_drops(),
        1,
        "the rejection must be counted against disabled traffic",
    );
    assert_eq!(
        logger.get_dropped(),
        0,
        "a disabled rejection is not a dispatch-admission drop",
    );
}

/// The same logger places filter and format calls once it is enabled.
#[test]
fn enabled_logger_runs_filters_and_formatting() {
    let (logger, filter_calls, format_calls) = counting_logger();

    assert_eq!(
        logger.log(FemtoLevel::Error, "delivered"),
        Some("formatted".to_owned()),
        "an enabled logger must return the formatted message",
    );
    assert_eq!(filter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(format_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        logger.get_disabled_drops(),
        0,
        "an enabled logger must not count disabled drops",
    );
}
