//! Unit tests for bounded exception capture.
//!
//! These tests drive the real capture entry points with small injected
//! [`CaptureLimits`]. They use real Python exceptions rather than synthetic
//! payloads, so the ceilings are exercised against the same object graph a
//! logging call sees in production.
//!
//! Assertions cover both halves of the contract: every retained field stays
//! inside its ceiling, and the truncation marker reports what was left out.
//!
//! Neighbouring modules cover what these tests deliberately do not. Stack
//! capture lives in [`super::traceback_capture_stack_limits_tests`], because it
//! needs a Python caller to exist and these tests raise their exceptions from
//! scripts instead of calling from Python. Containment of user `__str__` and
//! `__repr__` lives
//! in [`super::traceback_capture_containment_tests`], because asking how many
//! times user code ran needs a counter in the script's namespace, which would
//! obscure the payload shapes these tests are about. Exception-group ceilings
//! live in [`super::traceback_capture_group_tests`].

use pyo3::prelude::*;
use rstest::rstest;

use crate::exception_schema::{ExceptionPayload, StackFrame};
use crate::test_utils::capture_limit_helpers::{capture_raised, marker, tiny_limits};
use crate::traceback_capture::CaptureLimits;

#[rstest]
fn oversized_message_is_clamped_and_flagged() {
    Python::attach(|py| {
        let payload = capture_raised(py, c"raise ValueError('message ' * 100)", &tiny_limits());

        assert!(
            payload.message.len() <= 32,
            "message of {} bytes exceeded the ceiling",
            payload.message.len()
        );
        assert!(payload.message.ends_with("..."));
        assert!(marker(&payload).message_truncated);
    });
}

#[rstest]
fn many_arguments_are_counted_and_omitted() {
    Python::attach(|py| {
        let payload = capture_raised(py, c"raise ValueError(*range(1000))", &tiny_limits());

        assert_eq!(payload.args_repr.len(), 2);
        assert_eq!(marker(&payload).args_omitted, 998);
    });
}

#[rstest]
fn oversized_argument_is_clamped_before_retention() {
    Python::attach(|py| {
        let payload = capture_raised(py, c"raise ValueError('x' * 5000)", &tiny_limits());

        assert_eq!(payload.args_repr.len(), 1);
        let rendered = &payload.args_repr[0];
        assert!(
            rendered.len() <= 16,
            "argument repr of {} bytes exceeded the ceiling",
            rendered.len()
        );
        assert!(
            rendered.ends_with("...'"),
            "a cut literal should keep its closing quote: {rendered:?}"
        );
        assert!(marker(&payload).args_truncated);
    });
}

#[rstest]
fn huge_bytes_argument_is_summarised_without_materialising_it() {
    Python::attach(|py| {
        let payload = capture_raised(py, c"raise ValueError(b'\\xab' * 100000)", &tiny_limits());

        // A blob this large is described, not rendered: escaping every byte
        // would cost far more than the ceiling allows.
        assert_eq!(payload.args_repr.len(), 1);
        assert!(payload.args_repr[0].len() <= 16);
        assert!(
            payload.args_repr[0].ends_with("...'"),
            "a summarised blob should keep its closing quote: {:?}",
            payload.args_repr[0]
        );
        assert!(marker(&payload).args_truncated);
    });
}

#[rstest]
fn small_bytes_argument_keeps_bytes_literal_form() {
    Python::attach(|py| {
        let payload = capture_raised(py, c"raise ValueError(b'ab')", &tiny_limits());

        assert_eq!(payload.args_repr, vec!["b'ab'".to_string()]);
    });
}

#[rstest]
fn many_notes_are_counted_and_omitted() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
error = ValueError('notes')
for index in range(1000):
    error.add_note(f'note {index}')
raise error
",
            &tiny_limits(),
        );

        assert_eq!(payload.notes.len(), 2);
        // Only the probe position is confirmed to be a note; positions beyond
        // it are unclassified, so the count is a minimum, not a total.
        assert_eq!(marker(&payload).notes_omitted, 1);
    });
}

#[rstest]
fn deep_cause_chain_stops_at_the_depth_ceiling() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
error = ValueError('deepest')
for index in range(200):
    try:
        raise RuntimeError(f'layer {index}') from error
    except RuntimeError as wrapper:
        error = wrapper
raise error
",
            &tiny_limits(),
        );

        let mut depth = 0;
        let mut current = Some(&payload);
        while let Some(node) = current {
            depth += 1;
            current = node.cause.as_deref();
        }
        // The root plus one link per unit of `max_chain_depth`.
        assert_eq!(depth, 3);

        let tail = deepest_cause(&payload);
        assert!(marker(tail).cause_omitted);
        assert!(marker(&payload).budget_exhausted);
    });
}

/// Follow the cause chain to its deepest retained payload.
fn deepest_cause(payload: &ExceptionPayload) -> &ExceptionPayload {
    let mut current = payload;
    while let Some(cause) = current.cause.as_deref() {
        current = cause;
    }
    current
}

#[rstest]
fn chained_exception_keeps_descendants_free_of_args_and_notes() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
try:
    raise IOError('read failed')
except IOError as inner:
    inner.add_note('inner note')
    raise RuntimeError('wrapper') from inner
",
            &tiny_limits(),
        );
        let cause = payload.cause.as_deref().expect("cause should be present");

        // `raise ... from inner` also sets `__context__` to the same exception.
        // The cause already carries it, so nothing was lost and no marker should
        // claim otherwise.
        assert!(
            payload.truncation.is_none(),
            "the implicit duplicate context must not be reported as omitted: {:?}",
            payload.truncation
        );

        // Descendants never pay for argument or note rendering, because the
        // text formatter does not print them.
        assert!(cause.args_repr.is_empty());
        assert!(cause.notes.is_empty());
        assert_eq!(cause.message, "read failed");
    });
}

#[rstest]
fn self_referential_cause_is_dropped_as_a_cycle() {
    Python::attach(|py| {
        let payload = capture_raised(
            py,
            c"
error = ValueError('circular')
error.__cause__ = error
error.__context__ = error
raise error
",
            &tiny_limits(),
        );

        // Re-reading the root would recurse forever, so both links are dropped
        // and reported rather than followed.
        assert!(payload.cause.is_none());
        assert!(payload.context.is_none());
        assert!(marker(&payload).cause_omitted);
        assert!(marker(&payload).context_omitted);
    });
}

#[rstest]
fn aggregate_budget_stops_retention_and_marks_exhaustion() {
    Python::attach(|py| {
        let limits = CaptureLimits {
            max_notes: 40,
            max_note_bytes: 64,
            max_aggregate_bytes: 256,
            ..tiny_limits()
        };
        let payload = capture_raised(
            py,
            c"
error = ValueError('budget')
for index in range(40):
    error.add_note('note ' + 'x' * 40)
raise error
",
            &limits,
        );

        let retained = retained_text_bytes(&payload);
        assert!(
            retained <= 256,
            "retained {retained} bytes exceeded the aggregate allowance"
        );
        assert!(payload.notes.len() < 40);
        assert!(marker(&payload).budget_exhausted);
    });
}

/// Total text a payload retains, including every descendant.
///
/// This mirrors what capture charges to the aggregate budget: the type name and
/// module, the message, each argument and note, and each frame field. Summing
/// only notes and arguments, as an earlier version of the test did, would miss
/// an overrun in any other field the budget also pays for.
fn retained_text_bytes(payload: &ExceptionPayload) -> usize {
    let own = payload.type_name.len()
        + payload.module.as_deref().map_or(0, str::len)
        + payload.message.len()
        + payload.args_repr.iter().map(String::len).sum::<usize>()
        + payload.notes.iter().map(String::len).sum::<usize>()
        + payload.frames.iter().map(frame_text_bytes).sum::<usize>();
    let descendants = payload
        .cause
        .iter()
        .chain(payload.context.iter())
        .map(|nested| retained_text_bytes(nested))
        .sum::<usize>()
        + payload
            .exceptions
            .iter()
            .map(retained_text_bytes)
            .sum::<usize>();
    own + descendants
}

/// Text one frame charges to the aggregate budget.
fn frame_text_bytes(frame: &StackFrame) -> usize {
    frame.filename.len() + frame.function.len() + frame.source_line.as_deref().map_or(0, str::len)
}
