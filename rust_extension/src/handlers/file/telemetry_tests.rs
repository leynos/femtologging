//! Tests for how file-handler results map onto bounded wait outcomes.
//!
//! The Python-exposed diagnostics must never invent a counter for an
//! unclassified failure, so each public result path is pinned here before the
//! bindings record it.

use super::wait_outcomes::ShutdownOutcome;
use super::*;
use crate::handler::HandlerError;
use rstest::rstest;
use std::time::Duration;

#[rstest]
#[case::accepted(Ok(()), WaitOutcome::HandleAccepted)]
#[case::queue_full(Err(EnqueueFailure::QueueFull), WaitOutcome::HandleQueueFull)]
#[case::timed_out(
    Err(EnqueueFailure::TimedOut(Duration::from_millis(5))),
    WaitOutcome::HandleTimedOut
)]
#[case::closed(Err(EnqueueFailure::Closed), WaitOutcome::HandleClosed)]
#[case::disconnected(Err(EnqueueFailure::Disconnected), WaitOutcome::HandleDisconnected)]
fn classify_enqueue_maps_every_result(
    #[case] result: Result<(), EnqueueFailure>,
    #[case] expected: WaitOutcome,
) {
    assert_eq!(classify_enqueue(&result), expected);
}

#[rstest]
#[case::queue_full(EnqueueFailure::QueueFull, HandlerError::QueueFull)]
#[case::timed_out(
    EnqueueFailure::TimedOut(Duration::from_millis(5)),
    HandlerError::Timeout(Duration::from_millis(5))
)]
#[case::closed(EnqueueFailure::Closed, HandlerError::Closed)]
#[case::disconnected(EnqueueFailure::Disconnected, HandlerError::Closed)]
fn enqueue_failures_keep_the_public_error_contract(
    #[case] failure: EnqueueFailure,
    #[case] expected: HandlerError,
) {
    // The diagnostics distinguish a lost worker from a deliberate close, but
    // the public `handle()` contract has always reported both as closed.
    assert_eq!(failure.into_handler_error(), expected);
}

#[rstest]
#[case::acknowledged(FlushOutcome::Acknowledged, WaitOutcome::FlushAcknowledged)]
#[case::enqueue_timed_out(FlushOutcome::EnqueueTimedOut, WaitOutcome::FlushEnqueueTimedOut)]
#[case::ack_timed_out(FlushOutcome::AckTimedOut, WaitOutcome::FlushAckTimedOut)]
#[case::closed(FlushOutcome::Closed, WaitOutcome::FlushClosed)]
#[case::disconnected(FlushOutcome::Disconnected, WaitOutcome::FlushDisconnected)]
#[case::failed(FlushOutcome::Failed, WaitOutcome::FlushFailed)]
fn flush_outcomes_map_one_to_one(#[case] outcome: FlushOutcome, #[case] expected: WaitOutcome) {
    assert_eq!(WaitOutcome::from(outcome), expected);
}

#[rstest]
#[case::completed(ShutdownOutcome::Completed, WaitOutcome::CloseCompleted)]
#[case::timed_out(ShutdownOutcome::TimedOut, WaitOutcome::CloseTimedOut)]
#[case::panicked(ShutdownOutcome::Panicked, WaitOutcome::ClosePanicked)]
#[case::unknown(ShutdownOutcome::Unknown, WaitOutcome::CloseUnknown)]
fn shutdown_outcomes_map_one_to_one(
    #[case] outcome: ShutdownOutcome,
    #[case] expected: WaitOutcome,
) {
    assert_eq!(WaitOutcome::from(outcome), expected);
}

#[rstest]
#[case::drop(OverflowPolicy::Drop, "drop")]
#[case::block(OverflowPolicy::Block, "block")]
#[case::timeout(OverflowPolicy::Timeout(Duration::from_millis(5)), "timeout")]
fn policy_bucket_is_fixed_cardinality(
    #[case] policy: OverflowPolicy,
    #[case] expected: &'static str,
) {
    let mut cfg = TestConfig::new(std::io::Cursor::new(Vec::new()), DefaultFormatter);
    cfg.overflow_policy = policy;
    let handler = FemtoFileHandler::with_writer_for_test(cfg);

    assert_eq!(handler.policy_bucket(), expected);
}
