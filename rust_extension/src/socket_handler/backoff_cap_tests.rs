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
///
/// Both the bound and the sampled delay are checked. A floor-only assertion
/// would pass for an implementation that wraps `Duration::MAX` back to a small
/// value, so the bound must still be `Duration::MAX` after doubling, and the
/// millisecond sample must stay inside the clamped range.
#[rstest]
fn saturating_doubling_survives_very_large_durations() {
    let huge = Duration::MAX;
    let mut state = BackoffState::with_seed(policy(huge, huge, huge), 5);
    let start = Instant::now();
    let floor = Duration::from_millis(10);
    let millisecond_ceiling = Duration::from_millis(u64::MAX);

    let first = state.next_sleep(start).expect("first delay");
    assert_eq!(
        state.current_bound(),
        Duration::MAX,
        "the initial bound must be the base"
    );
    assert!(
        (floor..=millisecond_ceiling).contains(&first),
        "the first delay escaped the clamped range: {first:?}"
    );

    let second = state
        .next_sleep(start + Duration::from_millis(1))
        .expect("second delay");
    assert_eq!(
        state.current_bound(),
        Duration::MAX,
        "saturating doubling must not wrap to a smaller duration"
    );
    assert!(
        (floor..=millisecond_ceiling).contains(&second),
        "the second delay escaped the clamped range: {second:?}"
    );
}

/// A repeated later timestamp still doubles the bound.
///
/// Only the first failure timestamp is remembered. A second call at the same
/// later timestamp therefore doubles the bound again, so samples above the
/// first doubled bound become possible. With a base of 10 ms the repeated
/// call samples `10..=40`; across 64 seeds the chance that every sample stays
/// at or below 20 ms is negligible, and the test is deterministic for the
/// chosen seeds within one build.
#[rstest]
fn a_repeated_later_timestamp_still_doubles() {
    let policy = policy(
        Duration::from_millis(10),
        Duration::from_secs(60),
        Duration::from_secs(60),
    );
    let start = Instant::now();
    let later = start + Duration::from_millis(1);
    let mut saw_above_first_doubling = false;

    for seed in 0..64 {
        let mut state = BackoffState::with_seed(policy.clone(), seed);
        assert_eq!(
            state.next_sleep(start),
            Some(Duration::from_millis(10)),
            "the first delay is the base bound"
        );
        let first_doubled = state.next_sleep(later).expect("within the deadline");
        assert!(
            first_doubled <= Duration::from_millis(20),
            "the first doubling exceeded the bound: {first_doubled:?}"
        );
        let second_doubled = state.next_sleep(later).expect("within the deadline");
        assert!(
            (Duration::from_millis(10)..=Duration::from_millis(40)).contains(&second_doubled),
            "the repeated later timestamp exceeded the second bound: {second_doubled:?}"
        );
        saw_above_first_doubling |= second_doubled > Duration::from_millis(20);
    }

    assert!(
        saw_above_first_doubling,
        "a repeated later timestamp must widen the bound again"
    );
}

/// A bound above `u64::MAX` milliseconds clamps instead of truncating.
///
/// `u64::MAX + 1` whole milliseconds does not fit in a `u64`. Clamping to
/// `u64::MAX` leaves a range so wide that a floor-only result is implausible,
/// while truncating the `u128` back to `u64` would wrap to zero and always
/// sample the floor. The fixed seeds make the assertion deterministic within a
/// build without pinning a `StdRng` stream.
#[rstest]
fn the_millisecond_bound_saturates_at_u64_max() {
    let millisecond_cap = Duration::from_millis(u64::MAX) + Duration::from_millis(1);
    let mut state = BackoffState::with_seed(
        policy(millisecond_cap, millisecond_cap, Duration::from_secs(60)),
        7,
    );
    let start = Instant::now();
    let floor = Duration::from_millis(10);
    let ceiling = Duration::from_millis(u64::MAX);
    let mut saw_above_floor = false;

    for _ in 0..64 {
        let sleep = state.next_sleep(start).expect("within the deadline");
        assert!(
            (floor..=ceiling).contains(&sleep),
            "the clamped range escaped its bounds: {sleep:?}"
        );
        saw_above_floor |= sleep > floor;
    }

    assert!(
        saw_above_floor,
        "a truncated bound would sample only the floor"
    );
}
