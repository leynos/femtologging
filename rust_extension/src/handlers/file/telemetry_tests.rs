//! Tests for how file-handler results map onto bounded wait outcomes.
//!
//! The Python-exposed diagnostics must never invent a counter for an
//! unclassified failure, so each public result path is pinned here before the
//! bindings record it.

use super::*;
use crate::handler::HandlerError;
use rstest::rstest;
use std::time::Duration;

#[rstest]
#[case::accepted(Ok(()), WaitOutcome::HandleAccepted)]
#[case::queue_full(Err(HandlerError::QueueFull), WaitOutcome::HandleQueueFull)]
#[case::timed_out(
    Err(HandlerError::Timeout(Duration::from_millis(5))),
    WaitOutcome::HandleTimedOut
)]
#[case::closed(Err(HandlerError::Closed), WaitOutcome::HandleClosed)]
#[case::other(Err(HandlerError::Message("boom".to_owned())), WaitOutcome::HandleFailed)]
#[case::missing_context(Err(HandlerError::MissingContextDispatch), WaitOutcome::HandleFailed)]
fn classify_handle_maps_every_result(
    #[case] result: Result<(), HandlerError>,
    #[case] expected: WaitOutcome,
) {
    assert_eq!(classify_handle(&result), expected);
}

#[rstest]
#[case::acknowledged(FlushOutcome::Acknowledged, WaitOutcome::FlushAcknowledged)]
#[case::timed_out(FlushOutcome::TimedOut, WaitOutcome::FlushTimedOut)]
#[case::closed(FlushOutcome::Closed, WaitOutcome::FlushClosed)]
#[case::failed(FlushOutcome::Failed, WaitOutcome::FlushFailed)]
fn flush_outcomes_map_one_to_one(#[case] outcome: FlushOutcome, #[case] expected: WaitOutcome) {
    assert_eq!(WaitOutcome::from(outcome), expected);
}

#[rstest]
#[case::completed(ShutdownOutcome::Completed, WaitOutcome::CloseCompleted)]
#[case::timed_out(ShutdownOutcome::TimedOut, WaitOutcome::CloseTimedOut)]
#[case::panicked(ShutdownOutcome::Panicked, WaitOutcome::ClosePanicked)]
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
