//! Backoff deadline and reset contracts.
//!
//! Split from `backoff_tests.rs` so each module stays within the repository's
//! file-size limit. The tests seed [`BackoffState`] directly and assert state
//! transitions rather than literal random draws.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::super::super::{backoff::BackoffState, config::BackoffPolicy};
use super::policy;

/// The deadline refuses a delay at equality and allows one just below it.
#[rstest]
fn the_deadline_is_exclusive_of_equality() {
    let policy = policy(
        Duration::from_millis(10),
        Duration::from_millis(10),
        Duration::from_millis(100),
    );
    let mut state = BackoffState::with_seed(policy, 3);
    let start = Instant::now();

    assert!(state.next_sleep(start).is_some());
    assert!(
        state
            .next_sleep(start + Duration::from_millis(99))
            .is_some(),
        "a delay just inside the deadline must be produced"
    );
    assert_eq!(
        state.next_sleep(start + Duration::from_millis(100)),
        None,
        "a delay at the deadline must be refused"
    );
}

/// `record_success` resets once the failure window reaches `reset_after`.
///
/// After the reset the initial 10 ms bound returns exactly 10 ms. Without a
/// reset the bound would have doubled to 80 ms, and the chance that 64 seeds
/// all happen to sample the floor is negligible.
#[rstest]
fn record_success_resets_at_the_threshold() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(10),
        cap: Duration::from_secs(1),
        reset_after: Duration::from_millis(100),
        deadline: Duration::from_secs(60),
    };
    let start = Instant::now();

    for seed in 0..64 {
        let mut state = BackoffState::with_seed(policy.clone(), seed);
        let _ = state.next_sleep(start).expect("first delay");
        let _ = state
            .next_sleep(start + Duration::from_millis(10))
            .expect("second delay");
        let _ = state
            .next_sleep(start + Duration::from_millis(20))
            .expect("third delay");

        state.record_success(start + Duration::from_millis(100));
        let sleep = state
            .next_sleep(start + Duration::from_millis(101))
            .expect("delay after reset");
        assert_eq!(
            sleep,
            Duration::from_millis(10),
            "a reset must restore the initial bound at the threshold"
        );
    }
}

/// `reset_after_idle` measures idleness from the most recent success.
#[rstest]
fn reset_after_idle_resets_against_the_last_success() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(10),
        cap: Duration::from_secs(1),
        reset_after: Duration::from_millis(100),
        deadline: Duration::from_secs(60),
    };
    let start = Instant::now();

    for seed in 0..64 {
        let mut state = BackoffState::with_seed(policy.clone(), seed);
        let _ = state.next_sleep(start).expect("first delay");
        let _ = state
            .next_sleep(start + Duration::from_millis(10))
            .expect("second delay");
        let _ = state
            .next_sleep(start + Duration::from_millis(20))
            .expect("third delay");
        // Establish the last success without yet reaching the threshold.
        state.record_success(start + Duration::from_millis(50));

        state.reset_after_idle(start + Duration::from_millis(150));
        let sleep = state
            .next_sleep(start + Duration::from_millis(151))
            .expect("delay after reset");
        assert_eq!(
            sleep,
            Duration::from_millis(10),
            "idle reset must restore the initial bound"
        );
    }
}

/// A `record_success` reset must not reseed the random source.
///
/// Both states draw the same sequence before the reset. Afterwards the ranges
/// are identical because base and cap are equal, so equal draws prove the
/// reset left the random source untouched. A reseed would restart the stream
/// and make the post-reset sequences differ.
#[rstest]
fn record_success_reset_does_not_reseed() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(1000),
        cap: Duration::from_millis(1000),
        reset_after: Duration::from_millis(100),
        deadline: Duration::from_secs(60),
    };
    let start = Instant::now();
    let mut reset = BackoffState::with_seed(policy.clone(), 0xA11CE);
    let mut control = BackoffState::with_seed(policy, 0xA11CE);

    for step in 0..4_u64 {
        let now = start + Duration::from_millis(step * 10);
        assert_eq!(reset.next_sleep(now), control.next_sleep(now));
    }
    reset.record_success(start + Duration::from_millis(100));
    control.record_success(start + Duration::from_millis(50));

    for step in 0..8_u64 {
        let now = start + Duration::from_millis(200 + step * 10);
        assert_eq!(
            reset.next_sleep(now),
            control.next_sleep(now),
            "the reset state diverged at step {step}, so it reseeded"
        );
    }
}

/// An idle reset must also leave the random source untouched.
#[rstest]
fn reset_after_idle_does_not_reseed() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(1000),
        cap: Duration::from_millis(1000),
        reset_after: Duration::from_millis(100),
        deadline: Duration::from_secs(60),
    };
    let start = Instant::now();
    let mut reset = BackoffState::with_seed(policy.clone(), 0xB0B);
    let mut control = BackoffState::with_seed(policy, 0xB0B);

    for step in 0..4_u64 {
        let now = start + Duration::from_millis(step * 10);
        assert_eq!(reset.next_sleep(now), control.next_sleep(now));
    }
    reset.record_success(start + Duration::from_millis(50));
    control.record_success(start + Duration::from_millis(50));
    reset.reset_after_idle(start + Duration::from_millis(150));
    control.reset_after_idle(start + Duration::from_millis(149));

    for step in 0..8_u64 {
        let now = start + Duration::from_millis(200 + step * 10);
        assert_eq!(
            reset.next_sleep(now),
            control.next_sleep(now),
            "the idle reset diverged at step {step}, so it reseeded"
        );
    }
}
