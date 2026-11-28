//! Bounded representations for exception and `OSError` arguments.
//!
//! These tests cover values whose native representation would render a whole
//! subtree or ignore a writable field: nested exceptions, `OSError` with a
//! container `strerror`, and summaries that must not claim more truncation than
//! they caused.

use pyo3::prelude::*;

use crate::test_utils::capture_limit_helpers::{capture_raised, marker};
use crate::traceback_capture::CaptureLimits;

#[test]
fn an_os_error_with_a_container_strerror_stays_bounded() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise OSError(2, ['a', 'a'])",
            &CaptureLimits::default(),
        );

        assert_eq!(
            payload.message, "[Errno 2] ['a', 'a']",
            "the ordinary OSError form must be preserved"
        );
    });
}

#[test]
fn a_nested_builtin_exception_uses_a_summary() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise ValueError(ValueError('x' * 600))",
            &CaptureLimits::default(),
        );

        assert_eq!(
            payload.args_repr[0], "ValueError(...)",
            "an inherited exception repr must not render its arguments in full"
        );
    });
}

#[test]
fn a_dict_mutated_by_an_element_repr_is_still_rendered() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
container = {}

class Mutating:
    def __repr__(self):
        container['added'] = 1
        return 'm'

container['key'] = Mutating()
raise ValueError(container)
",
            &CaptureLimits::default(),
        );

        assert!(
            payload.args_repr[0].contains("'key': m"),
            "the snapshot must be rendered before the mutation is observed: {:?}",
            payload.args_repr[0]
        );
        assert!(
            !payload.args_repr[0].contains("added"),
            "the mutation must not appear in the snapshot: {:?}",
            payload.args_repr[0]
        );
    });
}

#[test]
fn an_os_error_uses_its_current_errno_and_strerror() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
error = OSError(2, ['old'])
error.errno = 13
error.strerror = ['new']
raise error
",
            &CaptureLimits::default(),
        );

        assert_eq!(
            payload.message, "[Errno 13] ['new']",
            "the writable fields are what OSError.__str__ formats"
        );
    });
}

#[test]
fn a_tuple_subclass_inheriting_tuple_repr_is_bounded() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class T(tuple):
    pass

raise ValueError(T(range(1000)))
",
            &CaptureLimits::default(),
        );

        assert!(
            payload.args_repr[0].len() < 200,
            "the inherited tuple repr must be bounded: {} bytes",
            payload.args_repr[0].len()
        );
    });
}

#[test]
fn a_container_subclass_with_custom_str_keeps_that_str() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class Named(list):
    def __str__(self):
        return 'named'

raise ValueError(Named([1]))
",
            &CaptureLimits::default(),
        );

        assert_eq!(payload.message, "named");
    });
}

#[test]
fn an_os_error_filename_survives_the_bounded_message() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise OSError(2, ['bad'], '/tmp/missing')",
            &CaptureLimits::default(),
        );

        assert_eq!(
            payload.message, "[Errno 2] ['bad']: '/tmp/missing'",
            "the bounded form must keep the filename"
        );
    });
}

#[test]
fn a_nested_exception_str_is_bounded() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise ValueError(ValueError([0] * 100000))",
            &CaptureLimits::default(),
        );

        assert!(
            payload.message.len() < 100,
            "the nested str must not render the inner list: {:?}",
            payload.message
        );
        assert_eq!(payload.message, "ValueError(...)");
        assert!(
            marker(&payload).message_truncated,
            "a substituted summary marks the message as shortened"
        );
    });
}

#[test]
fn an_oversized_blob_summary_does_not_claim_budget_exhaustion() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise ValueError(b'x' * 513)",
            &CaptureLimits::default(),
        );

        assert!(payload.message.contains("binary blob"));
        let marker = marker(&payload);
        assert!(marker.message_truncated);
        assert!(
            !marker.budget_exhausted,
            "a summary that fits the message ceiling is not aggregate exhaustion"
        );
    });
}
