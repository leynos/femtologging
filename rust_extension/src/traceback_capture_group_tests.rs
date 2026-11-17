//! Unit tests for bounded exception-group traversal.
//!
//! Exception groups have their own ceilings — width, depth and member identity
//! — so they are exercised apart from the per-field limits in
//! [`super::traceback_capture_limits_tests`]. Each case checks both halves of
//! the contract: the retained members are the prefix the ceiling allows, and
//! the marker reports the members that were left out.

use pyo3::prelude::*;
use rstest::rstest;

use crate::test_utils::capture_limit_helpers::{capture_raised, marker, tiny_limits};

#[rstest]
fn group_width_and_depth_are_enforced() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
members = [ValueError(str(index)) for index in range(6)]
raise ExceptionGroup('wide', members)
",
            &tiny_limits(),
        );

        assert_eq!(payload.exceptions.len(), 2);
        assert_eq!(marker(&payload).exceptions_omitted, 4);
        assert!(marker(&payload).members_truncated);
    });
}

#[rstest]
fn failed_exception_group_children_are_reported() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class BrokenGroup(ExceptionGroup):
    @property
    def exceptions(self):
        raise RuntimeError('children unavailable')

raise BrokenGroup('broken', [ValueError('child')])
",
            &tiny_limits(),
        );

        assert!(payload.exceptions.is_empty());
        assert_eq!(marker(&payload).exceptions_omitted, 1);
        assert_eq!(marker(&payload).failed_elements, 1);
        assert!(marker(&payload).members_truncated);
    });
}

#[rstest]
fn non_exception_group_children_are_skipped_and_reported() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
class MalformedGroup(ExceptionGroup):
    @property
    def exceptions(self):
        return [ValueError('retained'), object()]

raise MalformedGroup('malformed', [ValueError('constructor child')])
",
            &tiny_limits(),
        );

        assert_eq!(payload.exceptions.len(), 1);
        assert_eq!(payload.exceptions[0].type_name, "ValueError");
        assert_eq!(marker(&payload).exceptions_omitted, 1);
        assert_eq!(marker(&payload).failed_elements, 1);
        assert!(marker(&payload).members_truncated);
    });
}

#[rstest]
fn nested_groups_stop_at_the_depth_ceiling() {
    Python::attach(|py| {
        // Three levels of nesting against a ceiling of two. The root is at depth
        // zero, so the middle group is visited at depth one and the innermost at
        // depth two — where the check fires and its children are reported as
        // omitted instead of being descended into.
        let payload = capture_raised(
            py,
            c"
innermost = ExceptionGroup('innermost', [ValueError('leaf')])
middle = ExceptionGroup('middle', [innermost])
raise ExceptionGroup('outer', [middle])
",
            &tiny_limits(),
        );

        assert_eq!(payload.exceptions.len(), 1);
        let middle = &payload.exceptions[0];
        assert_eq!(middle.type_name, "ExceptionGroup");
        assert_eq!(middle.exceptions.len(), 1);

        let innermost = &middle.exceptions[0];
        assert_eq!(innermost.type_name, "ExceptionGroup");
        assert!(
            innermost.exceptions.is_empty(),
            "the depth ceiling should stop the descent"
        );
        assert_eq!(marker(innermost).exceptions_omitted, 1);
        assert!(marker(innermost).members_truncated);
        assert!(marker(&payload).budget_exhausted);
    });
}
