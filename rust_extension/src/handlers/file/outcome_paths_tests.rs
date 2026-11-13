//! Behavioural tests for the file handler's enqueue and flush outcomes.
//!
//! The mapping tests in `telemetry_tests` pin the enum conversions; these
//! tests drive the real enqueue and flush paths so a regression in the
//! classification itself is caught. Each test uses a scripted writer to hold
//! the worker at a known point, which keeps the assertions deterministic
//! without relying on timing luck.

use super::scripted_writer::{ScriptedWriter, Stage, expect_stage, handler_with, record};
use super::*;
use std::sync::mpsc;
use std::time::Duration;

/// A timeout policy reports a timed-out enqueue, not a full queue.
#[test]
fn a_timeout_policy_reports_a_timed_out_enqueue() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(
        writer.clone(),
        OverflowPolicy::Timeout(Duration::from_millis(20)),
    );

    handler
        .enqueue(record("first"))
        .expect("first record queued");
    expect_stage(&entered_rx, Stage::Write);
    handler
        .enqueue(record("second"))
        .expect("second record fills the queue");

    let failure = handler
        .enqueue(record("third"))
        .expect_err("a full queue must refuse the third record");
    assert!(
        matches!(failure, EnqueueFailure::TimedOut(_)),
        "a timeout policy must report a timeout, got {failure:?}",
    );
    assert_eq!(
        classify_enqueue(&Err(failure)),
        WaitOutcome::HandleTimedOut,
        "a timed-out enqueue must be counted as a timeout",
    );

    writer.release();
    handler.close();
}

/// A worker that dies while the handler is open reports a disconnection.
#[test]
fn a_dead_worker_reports_a_disconnected_enqueue_and_flush() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::panicking_writes(entered_tx);
    let handler = handler_with(writer, OverflowPolicy::Block);

    handler
        .enqueue(record("first"))
        .expect("the first record is queued before the worker dies");
    assert!(
        handler.completion.wait(Duration::from_secs(2)),
        "the panicking worker must still signal completion",
    );

    let failure = handler
        .enqueue(record("second"))
        .expect_err("a dead worker must refuse the record");
    assert_eq!(
        failure,
        EnqueueFailure::Disconnected,
        "a vanished channel is not a deliberate close",
    );
    assert_eq!(
        classify_enqueue(&Err(failure)),
        WaitOutcome::HandleDisconnected,
    );
    assert_eq!(
        handler.flush_outcome(),
        FlushOutcome::Disconnected,
        "a flush to a dead worker must report the disconnection",
    );
}

/// A flush that cannot be enqueued before the deadline is reported apart from
/// one the worker never answers.
#[test]
fn a_saturated_queue_reports_an_enqueue_timeout() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);

    handler
        .enqueue(record("first"))
        .expect("first record queued");
    expect_stage(&entered_rx, Stage::Write);
    handler
        .enqueue(record("second"))
        .expect("second record fills the queue");

    assert_eq!(
        handler.flush_outcome(),
        FlushOutcome::EnqueueTimedOut,
        "a flush that never reached the worker must not be reported as an ack timeout",
    );

    writer.release();
    handler.close();
}

/// A worker that accepts the flush but never answers reports an ack timeout.
#[test]
fn a_blocked_worker_reports_an_acknowledgement_timeout() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::blocking(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);

    assert_eq!(
        handler.flush_outcome(),
        FlushOutcome::AckTimedOut,
        "an unanswered flush must be reported as an ack timeout",
    );
    expect_stage(&entered_rx, Stage::Flush);

    writer.release();
    handler.close();
}

/// A worker that answers with an I/O failure is reported as a failed flush.
#[test]
fn a_failing_writer_reports_a_failed_flush() {
    let (entered_tx, _entered_rx) = mpsc::channel();
    let writer = ScriptedWriter::failing_flush(entered_tx);
    let handler = handler_with(writer.clone(), OverflowPolicy::Block);

    assert_eq!(
        handler.flush_outcome(),
        FlushOutcome::Failed,
        "an I/O failure must not be reported as a timeout",
    );

    writer.release();
    handler.close();
}
