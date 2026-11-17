//! Generated properties for the backoff bounds, deadline, and reset contracts.
//!
//! The named `rstest` cases pin individual boundary values. These properties
//! cover the ranges between them: arbitrary bases, caps, and timestamp
//! sequences, asserted against the state rather than against a particular
//! `StdRng` stream. Each case is cheap, and every seed is fixed by the
//! generated input, so a run is reproducible without assuming that rand keeps
//! a stable stream across versions.

use std::time::{Duration, Instant};

use proptest::prelude::*;

use super::super::super::{backoff::BackoffState, config::BackoffPolicy};
use super::policy;

/// The shortest delay the sampler may produce.
const MIN_SLEEP: Duration = Duration::from_millis(10);

/// Build a policy whose reset threshold is comfortably beyond these tests.
fn reset_policy(reset_after: Duration) -> BackoffPolicy {
    BackoffPolicy {
        base: MIN_SLEEP,
        cap: Duration::from_secs(1),
        reset_after,
        deadline: Duration::from_secs(60),
    }
}

proptest! {
    /// The first delay lies between the floor and the base bound.
    #[test]
    fn the_first_delay_stays_within_the_base(base_ms in 0u64..=5_000) {
        let base = Duration::from_millis(base_ms);
        let mut state = BackoffState::with_seed(
            policy(base, base.max(MIN_SLEEP), Duration::from_secs(60)),
            1,
        );
        let sleep = state.next_sleep(Instant::now()).expect("within the deadline");
        let lower = base.min(MIN_SLEEP);
        let upper = base.max(MIN_SLEEP);
        prop_assert!(
            (lower..=upper).contains(&sleep),
            "the first delay {sleep:?} escaped {lower:?}..={upper:?}"
        );
    }

    /// Each timestamp after the first doubles the bound, up to the cap.
    #[test]
    fn distinct_timestamps_double_the_bound_up_to_the_cap(
        base_ms in 10u64..=1_000,
        cap_multiplier in 1u64..=8,
        steps in 1u64..=10,
    ) {
        let base = Duration::from_millis(base_ms);
        let cap = base.saturating_mul(
            u32::try_from(cap_multiplier).expect("the multiplier fits in u32"),
        );
        let cap_ms = u64::try_from(cap.as_millis()).expect("the cap fits in u64");
        let mut state = BackoffState::with_seed(
            policy(base, cap, Duration::from_secs(60)),
            2,
        );
        let start = Instant::now();
        let mut expected_ms = base_ms;

        for step in 0..steps {
            let now = start + Duration::from_millis(step);
            let sleep = state.next_sleep(now).expect("within the deadline");
            prop_assert_eq!(
                state.current_bound(),
                Duration::from_millis(expected_ms),
                "step {} used the wrong bound",
                step
            );
            prop_assert!(
                (MIN_SLEEP..=Duration::from_millis(expected_ms)).contains(&sleep),
                "step {} sampled {sleep:?} outside 10 ms..={expected_ms} ms",
                step
            );
            expected_ms = expected_ms.saturating_mul(2).min(cap_ms);
        }
    }

    /// The deadline allows a call below it and refuses one at or beyond it.
    #[test]
    fn the_deadline_is_exclusive_of_equality(deadline_ms in 1u64..=10_000) {
        let deadline = Duration::from_millis(deadline_ms);
        let mut state = BackoffState::with_seed(
            policy(MIN_SLEEP, Duration::from_secs(1), deadline),
            3,
        );
        let start = Instant::now();

        prop_assert!(state.next_sleep(start).is_some());
        prop_assert!(
            state
                .next_sleep(start + deadline - Duration::from_millis(1))
                .is_some(),
            "a delay below the deadline must be produced"
        );
        prop_assert_eq!(
            state.next_sleep(start + deadline),
            None,
            "a delay at the deadline must be refused"
        );
        prop_assert_eq!(
            state.next_sleep(start + deadline + Duration::from_millis(1)),
            None,
            "a delay beyond the deadline must be refused"
        );
    }

    /// The idle reset fires exactly when idleness reaches `reset_after`.
    #[test]
    fn the_idle_reset_measures_from_the_last_success(
        reset_ms in 1u64..=1_000,
        success_fraction in 0u64..100,
        idle_fraction in 0u64..=199,
    ) {
        // Keep the success inside the window so `record_success` itself cannot
        // reset, and let the idle interval span both sides of the threshold.
        let success_ms = reset_ms * success_fraction / 100;
        let idle_ms = success_ms + reset_ms * idle_fraction / 100;
        let reset_after = Duration::from_millis(reset_ms);
        let mut state = BackoffState::with_seed(reset_policy(reset_after), 4);
        let start = Instant::now();

        let _ = state.next_sleep(start).expect("first delay");
        let _ = state
            .next_sleep(start + Duration::from_millis(1))
            .expect("second delay");
        prop_assert_eq!(state.current_bound(), Duration::from_millis(20));

        state.record_success(start + Duration::from_millis(success_ms));
        state.reset_after_idle(start + Duration::from_millis(idle_ms));

        let expected = if idle_ms - success_ms >= reset_ms {
            Duration::from_millis(10)
        } else {
            Duration::from_millis(20)
        };
        prop_assert_eq!(
            state.current_bound(),
            expected,
            "idle {} ms after a success {} ms into the window",
            idle_ms - success_ms,
            success_ms
        );
    }
}
