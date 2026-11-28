//! Bounded representations for containers and exception groups.
//!
//! `repr` of a container renders every element, and `repr` of an exception
//! group reproduces its whole subtree; shared child groups multiply that work
//! per level. These tests pin the bounded replacements: fixed group
//! placeholders, container entries rendered only up to explicit limits,
//! container subclasses that inherit native representations, and the same
//! treatment when such a value is a group member or a single argument.

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
