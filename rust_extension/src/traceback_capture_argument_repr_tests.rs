//! Bounded representations for container and exception-group arguments.
//!
//! `repr` of a container renders every element, and `repr` of an exception
//! group reproduces its whole subtree; shared child groups multiply that work
//! per level. These tests pin the bounded replacements: a fixed group
//! placeholder, container entries rendered only up to explicit limits, and the
//! same treatment when such a value is a group member or a single argument.

use std::ffi::CStr;

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rstest::rstest;

use crate::test_utils::capture_limit_helpers::{
    capture_raised, capture_raised_in, marker, tiny_limits,
};
use crate::traceback_capture::CaptureLimits;

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
fn a_large_container_is_rendered_from_a_bounded_prefix() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
repr_calls = 0

class Counted:
    def __repr__(self):
        global repr_calls
        repr_calls += 1
        return 'x'

raise ValueError([Counted() for _ in range(1000)])
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
        // The bounded prefix is rendered once for `args_repr` and once for the
        // message, which `str(list)` also produces, so two windows is the
        // expected cost; the thousand-entry container is never touched whole.
        assert!(
            calls <= 32,
            "container rendering represented {calls} entries"
        );
        assert!(
            payload.args_repr[0].len() < 1000,
            "the container representation must stay bounded: {} bytes",
            payload.args_repr[0].len()
        );
    });
}

#[test]
fn a_group_nested_in_a_container_keeps_the_placeholder() {
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

raise ValueError([ExceptionGroup('g', [Counted()])])
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
        assert_eq!(calls, 0, "a nested group must not represent its members");
        assert!(
            payload.args_repr[0].contains("<exception group>"),
            "the nested group must keep its placeholder: {:?}",
            payload.args_repr[0]
        );
    });
}

#[test]
fn a_tiny_ceiling_clamps_the_group_placeholder() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"raise ValueError(ExceptionGroup('g', [ValueError('child')]))",
            &tiny_limits(),
        );

        assert!(
            payload.args_repr[0].len() <= 16,
            "the placeholder must respect the argument ceiling: {:?}",
            payload.args_repr[0]
        );
        assert!(marker(&payload).args_truncated);
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

#[rstest]
#[case(c"raise ValueError((1,))", "(1,)")]
#[case(c"raise ValueError(set())", "set()")]
#[case(c"raise ValueError(frozenset())", "frozenset()")]
#[case(c"raise ValueError({'a': 1})", "{'a': 1}")]
fn small_containers_keep_their_python_forms(#[case] source: &CStr, #[case] expected: &str) {
    Python::attach(|py| {
        let payload = capture_raised(py, source, &CaptureLimits::default());

        assert_eq!(payload.args_repr[0], expected);
    });
}

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
fn a_list_subclass_inheriting_list_repr_is_bounded() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class Numbers(list):
    pass

raise ValueError(Numbers(range(1000)))
",
            &CaptureLimits::default(),
        );

        assert!(
            payload.args_repr[0].len() < 200,
            "the inherited list repr must be bounded: {} bytes",
            payload.args_repr[0].len()
        );
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
