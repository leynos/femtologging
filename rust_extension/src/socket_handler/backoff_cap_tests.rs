//! Backoff cap, doubling, and saturation contracts.
//!
//! Split from `backoff_tests.rs` so each module stays within the repository's
//! file-size limit. The tests seed [`BackoffState`] directly and assert bounds
//! rather than literal random draws.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::super::super::backoff::BackoffState;
use super::policy;

/// Successive failures stop growing once the cap is reached.
#[rstest]
fn doubling_stops_at_the_cap() {
    let cap = Duration::from_millis(30);
    let policy = policy(Duration::from_millis(10), cap, Duration::from_secs(60));
    let start = Instant::now();

    for seed in 0..64 {
        let mut state = BackoffState::with_seed(policy.clone(), seed);
        let _ = state.next_sleep(start).expect("first delay");
        let _ = state
            .next_sleep(start + Duration::from_millis(1))
            .expect("second delay");

        for step in 2..6_u64 {
            let now = start + Duration::from_millis(step);
            let sleep = state.next_sleep(now).expect("within the deadline");
            assert!(
                sleep <= cap,
                "the delay grew past the cap at step {step}: {sleep:?}"
            );
        }
    }
}

/// The first delay uses the base bound even when that exceeds the cap.
///
/// Capping only applies when the delay is doubled, so the base itself is the
/// first bound. The following failure applies the cap.
#[rstest]
fn the_first_delay_may_exceed_the_cap_when_the_base_does() {
    let base = Duration::from_millis(50);
    let cap = Duration::from_millis(20);
    let policy = policy(base, cap, Duration::from_secs(60));
    let start = Instant::now();
    let mut saw_above_cap = false;

    for seed in 0..64 {
        let mut state = BackoffState::with_seed(policy.clone(), seed);
        let first = state.next_sleep(start).expect("first delay");
        assert!(
            first <= base,
            "the first delay exceeded the base bound: {first:?}"
        );
        saw_above_cap |= first > cap;

        let later = state
            .next_sleep(start + Duration::from_millis(1))
            .expect("second delay");
        assert!(
            later <= cap,
            "a later delay must respect the cap: {later:?}"
        );
    }

    assert!(
        saw_above_cap,
        "the first delay must be able to exceed the cap"
    );
}

/// Doubling a very large duration saturates instead of overflowing.
#[rstest]
fn saturating_doubling_survives_very_large_durations() {
    let huge = Duration::MAX;
    let mut state = BackoffState::with_seed(policy(huge, huge, huge), 5);
    let start = Instant::now();

    let first = state.next_sleep(start).expect("first delay");
    assert!(first >= Duration::from_millis(10));
    let second = state
        .next_sleep(start + Duration::from_millis(1))
        .expect("second delay");
    assert!(second >= Duration::from_millis(10));
}

/// The millisecond bound saturates at `u64::MAX`, leaving a valid range.
#[rstest]
fn the_millisecond_bound_saturates_at_u64_max() {
    let nanosecond_cap = Duration::from_millis(u64::MAX);
    let mut state = BackoffState::with_seed(
        policy(nanosecond_cap, nanosecond_cap, Duration::from_secs(60)),
        7,
    );

    let sleep = state
        .next_sleep(Instant::now())
        .expect("within the deadline");
    assert!(
        sleep >= Duration::from_millis(10),
        "the clamped range must start at the minimum sleep"
    );
}
