//! Behavioural tests for `__notes__` collection.
//!
//! Notes live in an ordinary list that application code can replace with
//! anything, so these tests cover the degradation paths: a missing attribute, a
//! non-iterable value, mixed element types, and the extra iterator position that
//! exists only to prove a tail was omitted.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rstest::rstest;

use crate::test_utils::capture_limit_helpers::{capture_raised, marker, tiny_limits};
use crate::test_utils::traceback_test_helpers::create_value_error;
use crate::traceback_capture::capture_exception;

#[rstest]
fn capture_exception_without_notes_returns_empty_notes() {
    // Standard exceptions without __notes__ should return empty notes vector.
    // This tests the get_optional_attr degradation path for missing attributes.
    Python::attach(|py| {
        let exc = create_value_error(py, "test error").expect("exception should build");

        // Do not assert CPython internals/version-specific defaults for __notes__.
        // Only assert the capture output degrades to an empty notes vector.
        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed")
            .expect("payload should be Some");

        assert!(
            payload.notes.is_empty(),
            "notes should be empty when __notes__ is missing/None/empty"
        );
        assert_eq!(payload.type_name, "ValueError");
        assert!(
            payload.truncation.is_none(),
            "a missing __notes__ attribute is not a capture failure"
        );
    });
}

#[rstest]
fn capture_exception_with_non_iterable_notes_degrades_gracefully() {
    // Exceptions with a non-iterable __notes__ attribute should degrade gracefully.
    // This tests the type-mismatch degradation path for __notes__.
    Python::attach(|py| {
        let exc = create_value_error(py, "test error").expect("exception should build");

        // Set a malformed, non-iterable __notes__ value (integer instead of list)
        exc.setattr("__notes__", 123_i32)
            .expect("setting non-iterable __notes__ should succeed at Python level");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed even with malformed __notes__")
            .expect("payload should be Some");

        // For non-iterable __notes__, we expect graceful degradation to empty notes
        assert!(
            payload.notes.is_empty(),
            "expected empty notes for non-iterable __notes__, got: {:?}",
            payload.notes
        );
        assert_eq!(payload.type_name, "ValueError");
    });
}

#[rstest]
fn capture_exception_with_non_string_note_elements_degrades_gracefully() {
    // Exceptions with __notes__ containing non-string elements should still
    // be captured without failing, degrading notes content as needed.
    // Per ADR "partial extraction of collections" rule: non-string entries are
    // dropped/ignored but valid strings are preserved.
    Python::attach(|py| {
        let exc = create_value_error(py, "test error").expect("exception should build");

        // Create a list with mixed types: valid string, integer, bytes, None
        let code = c"
notes_list = ['valid note', 42, b'bytes note', None]
";
        let globals = PyDict::new(py);
        py.run(code, Some(&globals), None)
            .expect("code to create notes list should succeed");
        let notes_list = globals
            .get_item("notes_list")
            .expect("get_item should not fail")
            .expect("notes_list should exist");

        exc.setattr("__notes__", notes_list)
            .expect("setting list-valued __notes__ should succeed");

        let payload = capture_exception(py, &exc)
            .expect("capture_exception should succeed even with malformed note entries")
            .expect("payload should be Some");

        assert_eq!(payload.type_name, "ValueError");

        // Per ADR "partial extraction of collections" rule: only valid string
        // entries are preserved; non-strings are skipped.
        assert_eq!(
            payload.notes,
            vec!["valid note"],
            "only valid string notes should survive; non-strings should be skipped"
        );
    });
}

#[rstest]
fn a_note_probe_consumed_by_non_notes_reports_no_omission() {
    // The extra iterator position exists only to prove a tail. Here it is the
    // last real note, so nothing was omitted and the marker must not claim a
    // dropped note.
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
error = ValueError('note')
error.__notes__ = iter([None, 'first', 'second'])
raise error
",
            &tiny_limits(),
        );

        assert_eq!(payload.notes, vec!["first", "second"]);
        let omitted = payload.truncation.map_or(0, |marker| marker.notes_omitted);
        assert_eq!(omitted, 0, "both string notes were retained");
    });
}

#[rstest]
fn a_failing_note_iterator_is_not_reported_as_a_ceiling_omission() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class Noted:
    def __init__(self):
        self.step = 0

    def __iter__(self):
        return self

    def __next__(self):
        self.step += 1
        if self.step == 1:
            return 'first note'
        raise RuntimeError('iterator failed')

error = ValueError('note')
error.__notes__ = Noted()
raise error
",
            &tiny_limits(),
        );

        assert_eq!(payload.notes, vec!["first note"]);
        let marker = marker(&payload);
        assert_eq!(
            marker.notes_omitted, 0,
            "an iterator failure is not a drop caused by the note ceiling"
        );
        assert_eq!(
            marker.failed_elements, 1,
            "the failed iterator read must still be reported"
        );
    });
}

#[rstest]
fn a_scan_stopped_by_its_work_ceiling_is_reported() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
error = ValueError('note')
error.__notes__ = iter(['a', 'b', 0, 'c'])
raise error
",
            &tiny_limits(),
        );

        assert_eq!(payload.notes, vec!["a", "b"]);
        assert_eq!(
            marker(&payload).notes_omitted,
            1,
            "the unscanned tail must be reported as an incomplete scan"
        );
    });
}
