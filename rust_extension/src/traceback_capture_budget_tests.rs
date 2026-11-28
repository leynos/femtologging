//! Regression tests for aggregate capture and node budgets.
//!
//! Per-field limits are tested in [`super::traceback_capture_limits_tests`].
//! These cases check the shared ceilings that apply across the complete
//! exception graph, including fields whose individual limits would otherwise
//! allow them to fit.

use pyo3::prelude::*;
use rstest::rstest;

use crate::exception_schema::ExceptionPayload;
use crate::test_utils::capture_limit_helpers::{
    capture_raised, capture_raised_in, marker, tiny_limits,
};
use crate::traceback_capture::CaptureLimits;

#[rstest]
fn aggregate_budget_bounds_every_retained_string() {
    Python::attach(|py| {
        let limits = CaptureLimits {
            max_message_bytes: 64,
            max_aggregate_bytes: 20,
            ..tiny_limits()
        };
        let payload = capture_raised(py, c"raise ValueError('x' * 400)", &limits);

        assert!(
            retained_bytes(&payload) <= limits.max_aggregate_bytes,
            "payload retained {} bytes beyond the {} byte aggregate ceiling",
            retained_bytes(&payload),
            limits.max_aggregate_bytes
        );
        assert!(marker(&payload).message_truncated);
        assert!(marker(&payload).budget_exhausted);
    });
}

#[rstest]
fn exhausted_aggregate_budget_skips_unneeded_repr_calls() {
    Python::attach(|py| {
        let limits = CaptureLimits {
            max_message_bytes: 64,
            max_aggregate_bytes: 8,
            ..tiny_limits()
        };
        let globals = pyo3::types::PyDict::new(py);
        let payload = capture_raised_in(
            py,
            c"
calls = 0
class Hostile:
    def __repr__(self):
        global calls
        calls += 1
        return 'payload'
raise ValueError(Hostile())
",
            &limits,
            &globals,
        );

        assert!(retained_bytes(&payload) <= limits.max_aggregate_bytes);
        assert_eq!(payload.message, "");
        assert!(payload.args_repr.is_empty());
        assert_eq!(
            globals
                .get_item("calls")
                .expect("counter lookup should succeed")
                .expect("counter should be defined")
                .extract::<usize>()
                .expect("counter should be an integer"),
            0
        );
        assert_eq!(marker(&payload).args_omitted, 1);
        assert!(marker(&payload).budget_exhausted);
    });
}

#[rstest]
fn node_budget_includes_the_root_exception() {
    Python::attach(|py| {
        let limits = CaptureLimits {
            max_chain_depth: 8,
            max_nodes: 2,
            ..tiny_limits()
        };
        let payload = capture_raised(
            py,
            c"
leaf = ValueError('leaf')
middle = RuntimeError('middle')
middle.__cause__ = leaf
raise RuntimeError('outer') from middle
",
            &limits,
        );

        let cause = payload.cause.as_deref().expect("one cause should fit");
        assert!(cause.cause.is_none());
        assert!(marker(cause).cause_omitted);
        assert!(marker(&payload).budget_exhausted);
    });
}

/// Count every retained UTF-8 field across one exception graph.
fn retained_bytes(payload: &ExceptionPayload) -> usize {
    payload.type_name.len()
        + payload.module.as_ref().map_or(0, String::len)
        + payload.message.len()
        + payload.args_repr.iter().map(String::len).sum::<usize>()
        + payload.notes.iter().map(String::len).sum::<usize>()
        + payload
            .frames
            .iter()
            .map(|frame| {
                frame.filename.len()
                    + frame.function.len()
                    + frame.source_line.as_ref().map_or(0, String::len)
            })
            .sum::<usize>()
        + payload.cause.as_deref().map_or(0, retained_bytes)
        + payload.context.as_deref().map_or(0, retained_bytes)
        + payload.exceptions.iter().map(retained_bytes).sum::<usize>()
}
