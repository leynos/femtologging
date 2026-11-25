//! Performance-regression tests for percent format compilation.
//!
//! Rendered output cannot show whether a formatter recompiled its format or
//! rescanned an already examined suffix, so these tests read the test-only work
//! counters in [`instrumentation`]. A regression that recompiles while
//! rendering, bypasses the closing-parenthesis memo, or disables its cache
//! fails a unit test here instead of surfacing only in a benchmark.
//!
//! [`instrumentation`]: super::super::directives::instrumentation

use std::collections::BTreeMap;

use proptest::prelude::*;

use crate::{formatter::FemtoFormatter, level::FemtoLevel, log_record::FemtoLogRecord};

use super::super::{
    PercentFormatter,
    directives::{ClosingParenthesisSearch, compile, instrumentation},
};
use super::reference_format;

/// Compile `format` and return the work it performed.
fn measure_compilation(format: &str) -> instrumentation::Measurements {
    instrumentation::reset();
    let segments = compile(format);
    assert!(
        !segments.is_empty(),
        "every probe format compiles to at least one segment"
    );
    instrumentation::measurements()
}

/// Malformed starts that all reach the same closing parenthesis.
fn shared_close_format(starts: usize) -> String {
    format!("{}x)q", "%(".repeat(starts))
}

/// Malformed starts with no closing parenthesis anywhere.
fn missing_close_format(starts: usize) -> String {
    "%(".repeat(starts)
}

#[test]
fn rendering_reuses_the_segments_compiled_at_construction() {
    instrumentation::reset();
    let formatter = PercentFormatter::new("%(levelname)s %(levelno)d %(message)s", None);
    let record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");

    for _ in 0..256 {
        assert_eq!(formatter.format(&record), "INFO 2 hello");
    }

    assert_eq!(
        instrumentation::measurements().compilations,
        1,
        "construction must compile once and rendering must reuse the segments"
    );
}

/// Require that `format` compiled in one search and scanned a linear amount.
fn assert_single_search(format: &str, memo_explanation: &str) {
    let measurements = measure_compilation(format);

    assert_eq!(measurements.compilations, 1, "the probe compiles once");
    assert_eq!(measurements.delimiter_searches, 1, "{memo_explanation}");
    assert!(
        measurements.scanned_bytes <= format.len(),
        "scanned {} bytes of a {} byte format",
        measurements.scanned_bytes,
        format.len()
    );
}

#[test]
fn malformed_starts_sharing_a_closing_parenthesis_scan_it_once() {
    assert_single_search(
        &shared_close_format(64),
        "the memo must answer every later start from the first search",
    );
}

#[test]
fn malformed_starts_without_a_closing_parenthesis_scan_the_suffix_once() {
    assert_single_search(
        &missing_close_format(64),
        "a missing delimiter must be remembered for every later start",
    );
}

#[test]
fn malformed_start_work_grows_linearly_with_the_format() {
    let small = measure_compilation(&shared_close_format(64));
    let large = measure_compilation(&shared_close_format(128));

    assert!(
        large.scanned_bytes < small.scanned_bytes * 3,
        "doubling the starts must not quadruple the work: {} -> {} bytes",
        small.scanned_bytes,
        large.scanned_bytes
    );
}

#[test]
fn instrumentation_counts_fresh_scans_and_not_memo_hits() {
    instrumentation::reset();
    let mut search = ClosingParenthesisSearch::default();

    assert_eq!(search.find("(ab)c)", 0), Some(3));
    assert_eq!(search.find("(ab)c)", 2), Some(3));

    let measurements = instrumentation::measurements();
    assert_eq!(
        measurements.delimiter_searches, 1,
        "a memo hit must not be counted as a fresh scan"
    );
    assert_eq!(
        measurements.scanned_bytes, 4,
        "a fresh scan examines the suffix through the `)`"
    );
}

/// Arbitrary text biased towards percent-directive syntax.
fn directive_dense_format() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("%(message)s".to_owned()),
            Just("%(levelno)d".to_owned()),
            Just("%(ratio)f".to_owned()),
            Just("%(missing)s".to_owned()),
            Just("%%".to_owned()),
            Just("%".to_owned()),
            Just("%(".to_owned()),
            Just(")".to_owned()),
            Just(")q".to_owned()),
            Just("s".to_owned()),
            Just("d".to_owned()),
            Just("f".to_owned()),
            "[a-zA-Z0-9 ]{0,4}",
        ],
        0..32,
    )
    .prop_map(|tokens| tokens.concat())
}

proptest! {
    #[test]
    fn directive_dense_formats_render_like_the_reference_and_compile_linearly(
        format in directive_dense_format()
    ) {
        let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");
        record.metadata_mut().key_values = BTreeMap::from([
            ("ratio".to_owned(), "1.5".to_owned()),
            ("count".to_owned(), "7".to_owned()),
        ]);

        instrumentation::reset();
        let formatter = PercentFormatter::new(format.clone(), None);
        let measurements = instrumentation::measurements();

        prop_assert_eq!(
            formatter.format(&record),
            reference_format(&formatter, &format, &record),
        );
        prop_assert_eq!(measurements.compilations, 1);
        prop_assert!(
            measurements.scanned_bytes <= format.len(),
            "scanned {} bytes of a {} byte format",
            measurements.scanned_bytes,
            format.len()
        );
    }
}
