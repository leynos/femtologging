//! Unit tests for containment of user `__str__` and `__repr__` during capture.
//!
//! These tests are separated from the other bounded-capture tests in
//! [`super::traceback_capture_limits_tests`] because they ask a different
//! question. Those tests ask *what was retained*; these ask *what was run*, and
//! specifically whether capture contains failures and stops calling into user
//! code once a ceiling is reached rather than calling it and discarding the
//! result.
//!
//! The distinction matters because the two failures look identical from the
//! payload alone. An implementation that renders all 500 arguments and keeps
//! two produces exactly the payload a bounded one does, while still paying for
//! 500 calls into code the application controls — and one of those calls can
//! raise, block, or do anything else it likes. Counting the calls is the only
//! way to tell the two implementations apart, so the counting exception here
//! leaves its counter in the script's namespace where the test can read it.

use std::ffi::CStr;

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rstest::rstest;

use crate::test_utils::capture_limit_helpers::{
    capture_raised, capture_raised_in, marker, tiny_limits,
};
use crate::traceback_capture::CaptureLimits;

#[rstest]
fn a_raising_repr_is_skipped_without_failing_the_capture() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
repr_calls = 0
class Hostile:
    def __repr__(self):
        global repr_calls
        repr_calls += 1
        raise RuntimeError('no repr for you')

raise ValueError('valid', Hostile())
",
            &tiny_limits(),
            &globals,
        );

        let repr_calls: usize = globals
            .get_item("repr_calls")
            .expect("repr counter should exist")
            .expect("repr counter should not be None")
            .extract()
            .expect("repr counter should be an integer");
        assert_eq!(payload.args_repr, vec!["'valid'".to_string()]);
        assert_eq!(repr_calls, 1);
        assert_eq!(marker(&payload).failed_elements, repr_calls as u32);
        // The argument was skipped because its `__repr__` raised, not because a
        // ceiling dropped it, so it is reported once — as a failed conversion.
        assert_eq!(marker(&payload).args_omitted, 0);
    });
}

#[rstest]
fn repr_call_count_is_bounded_by_the_argument_ceiling() {
    Python::attach(|py| {
        // The counter is left in the script's own namespace, so the capture has
        // to run against that namespace rather than a private one.
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
repr_calls = 0

class Counting:
    def __repr__(self):
        global repr_calls
        repr_calls += 1
        return 'counting'

raise ValueError(*[Counting() for _ in range(500)])
",
            &tiny_limits(),
            &globals,
        );

        let calls: usize = globals
            .get_item("repr_calls")
            .expect("counter lookup should succeed")
            .expect("counter should be defined")
            .extract()
            .expect("counter should be an integer");

        assert_eq!(payload.args_repr.len(), 2);
        // Rendering every argument first and discarding the rest would make the
        // ceiling decorative; the call count proves the loop stops.
        assert_eq!(calls, 2);
    });
}

#[rstest]
fn a_huge_repr_result_is_clamped_before_retention() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
repr_calls = 0

class LargeRepr:
    def __repr__(self):
        global repr_calls
        repr_calls += 1
        return 'x' * 1000000

raise ValueError(LargeRepr())
",
            &tiny_limits(),
            &globals,
        );

        let calls: usize = globals
            .get_item("repr_calls")
            .expect("counter lookup should succeed")
            .expect("counter should be defined")
            .extract()
            .expect("counter should be an integer");

        assert!(payload.args_repr[0].len() <= 16);
        assert!(marker(&payload).args_truncated);
        assert_eq!(calls, 2);
    });
}

#[rstest]
#[case(c"raise ValueError(b'\\xab' * 100000)", "...'")]
#[case(c"raise ValueError(bytearray(b'x' * 100000))", "...')")]
fn large_binary_arguments_are_bounded_before_stringification(
    #[case] source: &CStr,
    #[case] suffix: &str,
) {
    Python::attach(|py| {
        let payload = capture_raised(py, source, &tiny_limits());

        assert!(payload.message.len() <= 32);
        assert!(
            payload.message.ends_with(suffix),
            "the summarised binary argument should keep its closing quote and wrapper: {:?}",
            payload.message
        );
        assert!(marker(&payload).message_truncated);
    });
}

#[test]
fn an_exception_group_argument_uses_a_placeholder_representation() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
repr_calls = 0

class Counted(Exception):
    def __repr__(self):
        global repr_calls
        repr_calls += 1
        return 'x' * 1000

raise ValueError(ExceptionGroup('g', [Counted()]))
",
            &CaptureLimits::default(),
            &globals,
        );

        let calls: usize = globals
            .get_item("repr_calls")
            .expect("counter lookup should succeed")
            .expect("counter should be defined")
            .extract()
            .expect("counter should be an integer");
        assert_eq!(
            payload.args_repr[0], "<exception group>",
            "a group argument must not be represented through its subtree"
        );
        assert_eq!(
            calls, 0,
            "representing a group must not run a member's repr"
        );
    });
}

#[test]
fn an_exception_group_does_not_repr_every_member() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
repr_calls = 0

class Counted(Exception):
    def __repr__(self):
        global repr_calls
        repr_calls += 1
        return 'x' * 1000

raise ExceptionGroup('g', [Counted(i) for i in range(100)])
",
            &tiny_limits(),
            &globals,
        );

        let calls: usize = globals
            .get_item("repr_calls")
            .expect("counter lookup should succeed")
            .expect("counter should be defined")
            .extract()
            .expect("counter should be an integer");
        // Only the members the group walker can retain are represented; the
        // caller's whole sequence is never handed to `repr`.
        assert!(
            calls <= 2,
            "capture represented {calls} members despite a width of 2"
        );
        assert!(
            payload.args_repr[1].len() <= 16,
            "the group member summary must respect the argument ceiling: {:?}",
            payload.args_repr[1]
        );
    });
}

#[test]
fn a_short_literal_that_expands_past_the_ceiling_keeps_its_quotes() {
    Python::attach(|py| {
        // The input is short enough to render inline, but escaping expands it
        // past the argument ceiling. Clipping it as plain text would cut the
        // final escape and drop the closing quote.
        let payload = capture_raised(py, c"raise ValueError(b'\\x00' * 12)", &tiny_limits());
        let argument = &payload.args_repr[0];

        assert!(
            argument.len() <= 16,
            "argument exceeded its ceiling: {argument:?}"
        );
        assert!(
            argument.ends_with('\''),
            "the clipped literal must keep its closing quote: {argument:?}"
        );
        assert!(
            !argument.ends_with('x') && !argument.ends_with('\\'),
            "the clipped literal must not end inside an escape: {argument:?}"
        );
    });
}

#[test]
fn a_bytearray_apostrophe_keeps_cpython_escaping() {
    Python::attach(|py| {
        // CPython's `bytearray` repr selects double quotes but still escapes an
        // apostrophe, unlike the shared `bytes` renderer.
        let payload = capture_raised(
            py,
            c"raise ValueError(bytearray(b\"a'\"))",
            &CaptureLimits::default(),
        );

        assert_eq!(
            payload.args_repr[0], "bytearray(b\"a\\'\")",
            "the inline bytearray must match CPython's representation"
        );
    });
}

#[rstest]
fn a_huge_note_is_sliced_before_it_is_retained() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"error = ValueError('note'); error.add_note('x' * 100000); raise error",
            &tiny_limits(),
        );

        assert_eq!(payload.notes.len(), 1);
        assert!(payload.notes[0].len() <= 16);
        assert!(payload.notes[0].ends_with("..."));
        assert!(marker(&payload).notes_truncated);
    });
}

#[rstest]
fn syntax_error_message_uses_its_message_attribute() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise SyntaxError('bad syntax', ('example.py', 1, 1, 'bad syntax'))",
            &tiny_limits(),
        );

        assert_eq!(payload.message, "bad syntax");
    });
}

#[rstest]
fn a_failing_group_child_attribute_is_contained_and_counted() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
class BrokenGroup(ExceptionGroup):
    @property
    def exceptions(self):
        global property_calls
        property_calls += 1
        raise RuntimeError('children unavailable')

property_calls = 0
raise BrokenGroup('broken', [ValueError('child')])
",
            &tiny_limits(),
            &globals,
        );

        let property_calls: usize = globals
            .get_item("property_calls")
            .expect("property counter should exist")
            .expect("property counter should not be None")
            .extract()
            .expect("property counter should be an integer");
        assert!(payload.exceptions.is_empty());
        assert_eq!(marker(&payload).exceptions_omitted, 1);
        assert_eq!(property_calls, 1);
        assert_eq!(marker(&payload).failed_elements, property_calls as u32);
        assert!(marker(&payload).members_truncated);
    });
}

#[rstest]
fn a_raising_str_is_contained_and_counted() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class Hostile(Exception):
    def __str__(self):
        raise RuntimeError('no string for you')

raise Hostile('ignored')
",
            &tiny_limits(),
        );

        assert_eq!(payload.message, "<exception str() failed>");
        assert_eq!(marker(&payload).failed_elements, 1);
    });
}
