//! Generated-time and diagnostic tests for the logger flush barrier.

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proptest::prelude::*;

use crate::handler::{FemtoHandlerTrait, HandlerError, capped_deadline, deadline_after, remaining};
use crate::log_record::FemtoLogRecord;

use super::super::flush_diagnostics::{FlushDiagnostics, FlushFailureStage, elapsed_bucket};
use super::{FemtoLogger, FlushOutcome, confirm_by_deadline, flush_handler_snapshot_at};

/// Handler with a fixed result for generated snapshot sequences.
struct ResultHandler {
    /// Whether this handler confirms its deadline-aware flush.
    confirmed: bool,
}

impl FemtoHandlerTrait for ResultHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
        self.confirmed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
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
        prop_assert!(remaining_budget <= budget);
        prop_assert_eq!(
            remaining_budget,
            budget.saturating_sub(Duration::from_millis(elapsed_ms)),
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
        let mut clock_samples = samples.into_iter();
        let outcome = flush_handler_snapshot_at(&snapshot, deadline, || {
            clock_samples.next().expect("one clock sample per boundary")
        });

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
        let outcome = FlushOutcome {
            result,
            deadline: Some(deadline),
            started_at,
            barrier_completed_at,
        };

        let confirmed = outcome.finish(
            caller_returned_at,
            &diagnostics,
            FlushFailureStage::CallerReturnDeadlineExceeded,
        );
        let expected = completed
            && barrier_elapsed_ms <= budget_ms
            && caller_elapsed_ms <= budget_ms;
        prop_assert_eq!(confirmed, expected);
        let snapshot = diagnostics.snapshot();
        prop_assert_eq!(snapshot.attempts, 1);
        prop_assert_eq!(snapshot.confirmed, u64::from(expected));
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

    assert_eq!(remaining(deadline, started_at), Duration::from_millis(50));
    assert_eq!(remaining(deadline, deadline), Duration::ZERO);
    assert_eq!(
        confirm_by_deadline(
            Ok(()),
            deadline,
            deadline,
            FlushFailureStage::BarrierDeadlineExceeded,
        ),
        Ok(())
    );
    assert_eq!(
        confirm_by_deadline(
            Ok(()),
            deadline,
            deadline + Duration::from_nanos(1),
            FlushFailureStage::BarrierDeadlineExceeded,
        ),
        Err(FlushFailureStage::BarrierDeadlineExceeded)
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
    assert_eq!(snapshot.attempts, 2);
    assert_eq!(snapshot.confirmed, 1);
    assert_eq!(
        snapshot.failures[FlushFailureStage::MarkerEnqueueTimedOut as usize],
        1,
    );
    assert_eq!(snapshot.elapsed_buckets[1], 1);
    assert_eq!(snapshot.elapsed_buckets[3], 1);
    assert_eq!(elapsed_bucket(Duration::from_secs(3)), 4);
}

#[test]
fn logger_flush_records_the_handler_failure_stage() {
    let logger = FemtoLogger::new("flush-diagnostics".to_owned());
    logger
        .add_handler(Arc::new(ResultHandler { confirmed: false }))
        .expect("diagnostic test handler should be accepted");

    assert!(!logger.flush_handlers());

    let snapshot = logger.flush_diagnostics.snapshot();
    assert_eq!(snapshot.attempts, 1);
    assert_eq!(snapshot.confirmed, 0);
    assert_eq!(
        snapshot.failures[FlushFailureStage::HandlerFlushUnconfirmed as usize],
        1,
    );
}
