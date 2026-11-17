//! Tests for the text markers that report bounded capture omissions.
//!
//! Each marker is checked by position as well as presence: a reader must see
//! the omission where the missing data would have appeared, not merely
//! somewhere in the output.

use crate::exception_schema::{ExceptionPayload, PayloadTruncation, StackFrame, StackTracePayload};
use crate::formatter::{format_exception_payload, format_stack_payload};

/// Build a one-frame payload so the position of a marker is observable.
fn single_frame_stack() -> StackTracePayload {
    StackTracePayload::new(vec![StackFrame::new("demo.py", 7, "main")])
}

/// Build an exception payload with a single frame.
fn single_frame_exception() -> ExceptionPayload {
    ExceptionPayload::new("ValueError", "boom")
        .with_frames(vec![StackFrame::new("demo.py", 7, "main")])
}

#[test]
fn stack_frame_omission_marker_precedes_retained_frames() {
    let stack = StackTracePayload {
        truncation: PayloadTruncation {
            frames_omitted: 2,
            ..Default::default()
        }
        .into_option(),
        ..single_frame_stack()
    };

    let output = format_stack_payload(&stack);

    let header = output
        .find("Stack (most recent call last):\n")
        .expect("stack header should be present");
    let marker = output
        .find("... 2 earlier frames omitted ...\n")
        .expect("frame omission marker should be present");
    let frame = output
        .find("demo.py")
        .expect("retained frame should be present");
    assert!(
        header < marker && marker < frame,
        "marker must sit between the header and the retained frames: {output}"
    );
}

#[test]
fn exception_frame_omission_marker_precedes_retained_frames() {
    let payload = single_frame_exception().with_truncation(PayloadTruncation {
        frames_omitted: 5,
        ..Default::default()
    });

    let output = format_exception_payload(&payload);

    let header = output
        .find("Traceback (most recent call last):\n")
        .expect("traceback header should be present");
    let marker = output
        .find("... 5 earlier frames omitted ...\n")
        .expect("frame omission marker should be present");
    let frame = output
        .find("demo.py")
        .expect("retained frame should be present");
    assert!(
        header < marker && marker < frame,
        "marker must sit between the header and the retained frames: {output}"
    );
}

#[test]
fn dropped_cause_and_context_render_their_own_markers() {
    let payload = ExceptionPayload::new("ValueError", "boom").with_truncation(PayloadTruncation {
        cause_omitted: true,
        context_omitted: true,
        ..Default::default()
    });

    let output = format_exception_payload(&payload);

    let cause = output
        .find("... exception cause omitted ...\n")
        .expect("cause marker should be present");
    let context = output
        .find("... exception context omitted ...\n")
        .expect("context marker should be present");
    assert!(
        cause < context,
        "cause must render before context: {output}"
    );
}

#[test]
fn displayed_cause_does_not_render_an_omitted_cause_marker() {
    let cause = ExceptionPayload::new("OSError", "root cause");
    let payload = ExceptionPayload::new("ValueError", "boom")
        .with_cause(cause)
        .with_truncation(PayloadTruncation {
            cause_omitted: true,
            ..Default::default()
        });

    let output = format_exception_payload(&payload);

    assert!(output.contains("OSError: root cause"));
    assert!(
        !output.contains("... exception cause omitted ..."),
        "a retained cause must not also be reported as omitted: {output}"
    );
}

#[test]
fn retained_context_does_not_hide_an_omitted_cause() {
    let context = ExceptionPayload::new("OSError", "outer context");
    let payload = ExceptionPayload::new("ValueError", "boom")
        .with_context(context)
        .with_truncation(PayloadTruncation {
            cause_omitted: true,
            ..Default::default()
        });

    let output = format_exception_payload(&payload);

    assert!(output.contains("OSError: outer context"));
    assert!(
        output.contains("... exception cause omitted ..."),
        "the missing cause must still be reported beside a retained context: {output}"
    );
}

#[test]
fn suppressed_displayed_context_still_reports_the_omission() {
    let context = ExceptionPayload::new("OSError", "ignored context");
    let mut payload = ExceptionPayload::new("ValueError", "boom")
        .with_context(context)
        .with_truncation(PayloadTruncation {
            context_omitted: true,
            ..Default::default()
        });
    payload.suppress_context = true;

    let output = format_exception_payload(&payload);

    assert!(
        !output.contains("ignored context"),
        "suppressed context must not be rendered: {output}"
    );
    assert!(
        output.contains("... exception context omitted ..."),
        "the omission marker must explain the missing context: {output}"
    );
}

#[test]
fn group_omission_marker_reports_dropped_members() {
    let payload =
        ExceptionPayload::new("ExceptionGroup", "several").with_truncation(PayloadTruncation {
            exceptions_omitted: 4,
            ..Default::default()
        });

    let output = format_exception_payload(&payload);

    assert!(
        output.contains("  ... and 4 more exceptions ...\n"),
        "group marker should report the dropped members: {output}"
    );
}

#[test]
fn summary_reports_every_omission_without_a_structural_marker() {
    let payload = ExceptionPayload::new("ValueError", "boom").with_truncation(PayloadTruncation {
        message_truncated: true,
        args_omitted: 2,
        args_truncated: true,
        notes_omitted: 3,
        notes_truncated: true,
        failed_elements: 4,
        members_truncated: true,
        budget_exhausted: true,
        ..Default::default()
    });

    let output = format_exception_payload(&payload);

    let expected = concat!(
        "... exception payload truncated (message truncated; 2 arguments omitted; ",
        "argument representations truncated; 3 notes omitted; notes truncated; ",
        "4 elements omitted after conversion failures; exception group members truncated; ",
        "capture budget exhausted) ...\n"
    );
    assert!(
        output.ends_with(expected),
        "summary should be the final line in a fixed order: {output}"
    );
}

#[test]
fn payload_without_a_marker_renders_no_truncation_text() {
    let exception = single_frame_exception();
    let stack = single_frame_stack();

    let exception_output = format_exception_payload(&exception);
    let stack_output = format_stack_payload(&stack);

    for output in [&exception_output, &stack_output] {
        assert!(
            !output.contains("omitted") && !output.contains("truncated"),
            "an untruncated payload must not invent markers: {output}"
        );
    }
}
