//! Generated-time and diagnostic tests for the logger flush barrier.

use std::any::Any;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proptest::prelude::*;

use crate::handler::{
    FemtoHandlerTrait, FlushContext, HandlerError, capped_deadline, deadline_after, remaining,
};
use crate::log_record::FemtoLogRecord;

use crate::monotonic_clock::MonotonicClock;

use super::super::flush_clock::FlushDeadline;
use super::super::flush_diagnostics::{FlushDiagnostics, FlushFailureStage, elapsed_bucket};
use super::{FemtoLogger, FlushOutcome, confirm_by_deadline, flush_handler_snapshot_at};

const TEST_FLUSH_BUDGET: Duration = Duration::from_secs(1);

/// Clock that returns a fixed sample for a deadline-orchestration test.
struct FixedClock {
    /// The monotonic sample every stage observes.
    now: Instant,
}

impl MonotonicClock for FixedClock {
    fn now(&self) -> Instant {
        self.now
    }
}

/// Clock with explicit samples for boundary tests.
struct SequenceClock {
    /// Samples returned in the order the production barrier requests them.
    samples: Mutex<VecDeque<Instant>>,
    /// Time returned after the explicit samples have been consumed.
    fallback: Instant,
}

impl MonotonicClock for SequenceClock {
    fn now(&self) -> Instant {
        self.samples
            .lock()
            .expect("clock sample lock should not be poisoned")
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

/// Handler with a fixed result for generated snapshot sequences.
struct ResultHandler {
    /// Whether this handler confirms its deadline-aware flush.
    confirmed: bool,
}

impl FemtoHandlerTrait for ResultHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _context: &crate::handler::FlushContext) -> bool {
        self.confirmed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn assert_single_flush_failure(diagnostics: &FlushDiagnostics, stage: FlushFailureStage) {
    let snapshot = diagnostics.snapshot();
    assert_eq!(
        (
            snapshot.attempts,
            snapshot.confirmed,
            snapshot.failures[stage as usize],
        ),
        (1, 0, 1),
        "one unconfirmed flush should be counted at its terminal stage",
    );
}

proptest! {
    #[test]
    fn generated_handler_sequences_cannot_extend_the_absolute_deadline(
        budget_ms in 1_u64..=5_000,
        elapsed_ms in 0_u64..=10_000,
        local_budget_ms in 0_u64..=10_000,
        handlers in prop::collection::vec((any::<bool>(), 0_u64..=2_000), 0..7),
    ) {
        let start = Instant::now();
        let budget = Duration::from_millis(budget_ms);
        let deadline = deadline_after(start, budget).expect("small budgets are representable");
        let current = start + Duration::from_millis(elapsed_ms);

        let remaining_budget = remaining(deadline, current);
        prop_assert_eq!(
            (remaining_budget <= budget, remaining_budget),
            (
                true,
                budget.saturating_sub(Duration::from_millis(elapsed_ms)),
            ),
        );

        let local_cap = capped_deadline(
            deadline,
            current,
            Duration::from_millis(local_budget_ms),
        );
        prop_assert!(local_cap <= deadline);

        let snapshot = handlers
            .iter()
            .map(|(confirmed, _)| {
                Arc::new(ResultHandler { confirmed: *confirmed }) as Arc<dyn FemtoHandlerTrait>
            })
            .collect::<Vec<_>>();
        let mut samples = Vec::with_capacity(handlers.len() * 2 + 1);
        let mut simulated_now = start;
        samples.push(simulated_now);
        for (_, spent_ms) in &handlers {
            simulated_now += Duration::from_millis(*spent_ms);
            samples.push(simulated_now);
            samples.push(simulated_now);
        }
        let clock = Arc::new(SequenceClock {
            samples: Mutex::new(samples.into_iter().collect()),
            fallback: deadline,
        });
        let context = FlushContext::with_clock(start, Some(deadline), clock);
        let outcome = flush_handler_snapshot_at(&snapshot, &context);

        let mut expected = true;
        let mut spent_ms = 0_u64;
        for (confirmed, handler_spent_ms) in &handlers {
            if spent_ms >= budget_ms || !confirmed {
                expected = false;
                break;
            }
            spent_ms += handler_spent_ms;
            if spent_ms > budget_ms {
                expected = false;
                break;
            }
        }
        prop_assert_eq!(outcome.is_ok(), expected);
    }

    #[test]
    fn flush_outcome_requires_success_before_both_deadline_checks(
        budget_ms in 1_u64..=5_000,
        barrier_elapsed_ms in 0_u64..=7_000,
        caller_elapsed_ms in 0_u64..=7_000,
        completed in any::<bool>(),
    ) {
        let started_at = Instant::now();
        let deadline = deadline_after(started_at, Duration::from_millis(budget_ms))
            .expect("small budgets are representable");
        let barrier_completed_at = started_at + Duration::from_millis(barrier_elapsed_ms);
        let caller_returned_at = started_at + Duration::from_millis(caller_elapsed_ms);
        let result = if completed {
            Ok(())
        } else {
            Err(FlushFailureStage::HandlerFlushUnconfirmed)
        };
        let diagnostics = FlushDiagnostics::default();
        let timing = FlushDeadline::with_clock(
            Duration::from_millis(budget_ms),
            Arc::new(FixedClock { now: started_at }),
        );
        let sampled_deadline = timing.instant();
        prop_assert_eq!(sampled_deadline, Some(deadline));
        let outcome = FlushOutcome {
            result,
            timing,
            barrier_completed_at,
        };

        let confirmed = outcome.finish_at(
            caller_returned_at,
            &diagnostics,
            FlushFailureStage::CallerReturnDeadlineExceeded,
        );
        let expected = completed
            && barrier_elapsed_ms <= budget_ms
            && caller_elapsed_ms <= budget_ms;
        prop_assert_eq!((sampled_deadline, confirmed), (Some(deadline), expected));
        let snapshot = diagnostics.snapshot();
        prop_assert_eq!((snapshot.attempts, snapshot.confirmed), (1, u64::from(expected)));
    }
}

#[test]
fn flush_deadline_helpers_fail_closed_on_overflow() {
    let start = Instant::now();
    assert_eq!(
        deadline_after(start, Duration::MAX),
        None,
        "an unrepresentable monotonic deadline must be rejected"
    );
}

#[test]
fn flush_deadline_helpers_cover_the_exact_boundary() {
    let started_at = Instant::now();
    let deadline = deadline_after(started_at, Duration::from_millis(50))
        .expect("a short test deadline should be representable");

    let confirmed_at_deadline = confirm_by_deadline(
        Ok(()),
        deadline,
        deadline,
        FlushFailureStage::BarrierDeadlineExceeded,
    );
    let confirmed_after_deadline = confirm_by_deadline(
        Ok(()),
        deadline,
        deadline + Duration::from_nanos(1),
        FlushFailureStage::BarrierDeadlineExceeded,
    );
    assert_eq!(
        (
            remaining(deadline, started_at),
            remaining(deadline, deadline),
            confirmed_at_deadline,
            confirmed_after_deadline,
        ),
        (
            Duration::from_millis(50),
            Duration::ZERO,
            Ok(()),
            Err(FlushFailureStage::BarrierDeadlineExceeded),
        ),
        "deadline helpers must preserve the exact completion boundary",
    );
}

#[test]
fn flush_diagnostics_use_fixed_stage_and_elapsed_buckets() {
    let diagnostics = FlushDiagnostics::default();
    diagnostics.record(
        Err(FlushFailureStage::MarkerEnqueueTimedOut),
        Duration::from_millis(50),
    );
    diagnostics.record(Ok(()), Duration::from_millis(750));

    let snapshot = diagnostics.snapshot();
    assert_eq!(
        (
            snapshot.attempts,
            snapshot.confirmed,
            snapshot.failures[FlushFailureStage::MarkerEnqueueTimedOut as usize],
            snapshot.elapsed_buckets,
        ),
        (2, 1, 1, [0, 1, 0, 1, 0]),
        "diagnostics must preserve outcome stages and elapsed buckets",
    );
    assert_eq!(elapsed_bucket(Duration::from_secs(3)), 4);

    let metrics = snapshot.metrics();
    assert_eq!(
        (
            metrics.len(),
            metrics["flush_handlers_attempts"],
            metrics["flush_handlers_confirmed"],
            metrics["flush_handlers_failure_marker_enqueue_timed_out"],
            metrics["flush_handlers_elapsed_gt_10_le_100_ms"],
            metrics["flush_handlers_elapsed_gt_500_le_2000_ms"],
        ),
        (22, 2, 1, 1, 1, 1),
        "metrics must expose the stable key set and recorded counts",
    );
}

#[test]
fn elapsed_buckets_use_exact_duration_boundaries() {
    let cases = [
        (Duration::ZERO, 0),
        (Duration::from_millis(10), 0),
        (Duration::from_millis(10) + Duration::from_nanos(1), 1),
        (Duration::from_millis(100), 1),
        (Duration::from_millis(100) + Duration::from_nanos(1), 2),
        (Duration::from_millis(500), 2),
        (Duration::from_millis(500) + Duration::from_nanos(1), 3),
        (Duration::from_millis(2_000), 3),
        (Duration::from_millis(2_000) + Duration::from_nanos(1), 4),
        (Duration::from_secs(3), 4),
    ];

    for (elapsed, expected) in cases {
        assert_eq!(
            elapsed_bucket(elapsed),
            expected,
            "elapsed {elapsed:?} must enter its disjoint range, not a truncated one",
        );
    }
}

#[test]
fn logger_flush_records_the_handler_failure_stage() {
    let logger = FemtoLogger::new("flush-diagnostics".to_owned())
        .expect("diagnostic test logger should be constructible");
    logger
        .add_handler(Arc::new(ResultHandler { confirmed: false }))
        .expect("diagnostic test handler should be accepted");

    assert!(!logger.flush_handlers());

    assert_single_flush_failure(
        &logger.flush_diagnostics,
        FlushFailureStage::HandlerFlushUnconfirmed,
    );
    let metrics = logger.flush_diagnostics.snapshot().metrics();
    assert_eq!(
        metrics["flush_handlers_failure_handler_flush_unconfirmed"],
        1,
    );
}

#[path = "flush_tests_clock.rs"]
mod clock;

#[path = "flush_handshake_property_tests.rs"]
mod handshake_property_tests;
