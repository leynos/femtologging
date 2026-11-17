//! Serialization tests for the bounded truncation marker.
//!
//! [`PayloadTruncation`] is a *report*, not data: every field is a count or a
//! flag, so a payload stays a fixed size however much was discarded. That makes
//! two properties worth pinning down separately from the general schema
//! round-trips in [`super::schema_tests`]:
//!
//! - a field that saw no omission must be absent from the wire form, because
//!   padding a marker with zeroes would defeat the point of having it;
//! - the marker must survive a round trip through both JSON and MessagePack
//!   without acquiring or losing a field.
//!
//! The marker reaches Python as part of an exception or stack payload, so a
//! field that failed to round-trip would silently read as "nothing omitted" —
//! the one wrong answer a consumer cannot detect.

use crate::exception_schema::*;
use rstest::rstest;

#[rstest]
fn truncation_marker_omits_default_fields() {
    let marker = PayloadTruncation {
        frames_omitted: 3,
        budget_exhausted: true,
        ..Default::default()
    };
    let json = serde_json::to_string(&marker).expect("serialize");

    assert!(json.contains("frames_omitted"));
    assert!(json.contains("budget_exhausted"));
    // A marker is a fixed-size record, so fields that saw no omission must not
    // appear at all rather than padding the payload with zeroes.
    assert!(!json.contains("args_omitted"));
    assert!(!json.contains("notes_truncated"));
    assert!(!json.contains("message_truncated"));
}

#[rstest]
fn truncation_marker_json_round_trip() {
    let marker = PayloadTruncation {
        message_truncated: true,
        args_omitted: 7,
        args_truncated: true,
        notes_omitted: 2,
        notes_truncated: true,
        frames_omitted: 5,
        cause_omitted: true,
        context_omitted: true,
        exceptions_omitted: 4,
        members_truncated: true,
        failed_elements: 3,
        budget_exhausted: true,
    };

    let json = serde_json::to_string(&marker).expect("serialize marker");
    let decoded: PayloadTruncation = serde_json::from_str(&json).expect("deserialize marker");
    assert_eq!(marker, decoded);
}

#[rstest]
fn exception_payload_truncation_round_trip() {
    let payload = ExceptionPayload::new("Error", "msg").with_truncation(PayloadTruncation {
        frames_omitted: 1,
        ..Default::default()
    });

    let json = serde_json::to_string(&payload).expect("serialize");
    let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(payload, decoded);
}

#[rstest]
fn stack_trace_payload_truncation_round_trip() {
    let payload = StackTracePayload {
        truncation: Some(PayloadTruncation {
            frames_omitted: 9,
            budget_exhausted: true,
            ..Default::default()
        }),
        ..StackTracePayload::new(vec![StackFrame::new("a.py", 1, "main")])
    };

    let json = serde_json::to_string(&payload).expect("serialize");
    let decoded: StackTracePayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(payload, decoded);
}

#[rstest]
fn empty_truncation_marker_is_dropped() {
    let payload =
        ExceptionPayload::new("Error", "msg").with_truncation(PayloadTruncation::default());
    assert!(payload.truncation.is_none());
    assert!(PayloadTruncation::default().is_empty());
}

#[rstest]
fn truncation_marker_merge_combines_counts_and_flags() {
    let mut base = PayloadTruncation {
        frames_omitted: 2,
        cause_omitted: true,
        ..Default::default()
    };
    base.merge(&PayloadTruncation {
        frames_omitted: 3,
        context_omitted: true,
        budget_exhausted: true,
        ..Default::default()
    });

    assert_eq!(base.frames_omitted, 5);
    assert!(base.cause_omitted);
    assert!(base.context_omitted);
    assert!(base.budget_exhausted);
    assert!(!base.is_empty());
}
