//! Backoff jitter, cap, deadline, and reset contracts.
//!
//! These tests seed [`BackoffState`] directly so they can assert the
//! application's invariants without assuming a particular `StdRng` stream.
//! Random streams are not guaranteed to be identical across rand versions, so
//! the assertions here are about ranges, bounds, and state transitions rather
//! than literal draw values.
//!
//! The cap and reset cases live in sibling modules so no single file outgrows
//! the repository's module-size limit.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::super::{backoff::BackoffState, config::BackoffPolicy};

/// Build a policy whose reset threshold is comfortably beyond these tests.
fn policy(base: Duration, cap: Duration, deadline: Duration) -> BackoffPolicy {
    BackoffPolicy {
        base,
        cap,
        deadline,
        ..BackoffPolicy::default()
    }
}

/// A sub-millisecond bound truncates to zero milliseconds and takes the floor.
#[rstest]
#[case(Duration::ZERO)]
#[case(Duration::from_micros(999))]
fn sub_millisecond_bounds_yield_the_minimum_sleep(#[case] base: Duration) {
    let mut state = BackoffState::with_seed(policy(base, base, Duration::from_secs(1)), 7);

    assert_eq!(
        state.next_sleep(Instant::now()),
        Some(Duration::from_millis(10)),
        "a bound below one millisecond must use the minimum sleep"
    );
}

/// Bounds at or below the minimum are returned exactly, without sampling.
#[rstest]
#[case(1)]
#[case(9)]
#[case(10)]
fn bounds_up_to_the_minimum_are_returned_exactly(#[case] base_ms: u64) {
    let base = Duration::from_millis(base_ms);
    let mut state = BackoffState::with_seed(policy(base, base, Duration::from_secs(1)), 11);

    assert_eq!(state.next_sleep(Instant::now()), Some(base));
}

/// An inclusive bound of 11 ms must yield both 10 and 11 over a fixed sample.
///
/// Each draw is independent and uniform over two values, so the chance of
/// missing one endpoint in 64 draws is about `2 * (1/2)^64`, which is
/// negligible. The test is still deterministic for a fixed seed and build.
#[rstest]
fn an_eleven_millisecond_bound_includes_both_endpoints() {
    let bound = Duration::from_millis(11);
    let mut state = BackoffState::with_seed(policy(bound, bound, Duration::from_secs(1)), 0x5EED);
    let start = Instant::now();
    let (mut saw_low, mut saw_high) = (false, false);

    for _ in 0..64 {
        let sleep = state.next_sleep(start).expect("within the deadline");
        assert!(
            sleep == Duration::from_millis(10) || sleep == bound,
            "jitter escaped the inclusive bound: {sleep:?}"
        );
        saw_low |= sleep == Duration::from_millis(10);
        saw_high |= sleep == bound;
    }

    assert!(
        saw_low && saw_high,
        "both endpoints must occur: low={saw_low}, high={saw_high}"
    );
}

/// A larger bound keeps every sample between the floor and the bound.
#[rstest]
fn jitter_stays_within_a_larger_bound() {
    let bound = Duration::from_secs(1);
    let mut state = BackoffState::with_seed(policy(bound, bound, Duration::from_secs(1)), 42);
    let start = Instant::now();

    for _ in 0..64 {
        let sleep = state.next_sleep(start).expect("within the deadline");
        assert!(
            (Duration::from_millis(10)..=bound).contains(&sleep),
            "jitter escaped the configured bound: {sleep:?}"
        );
    }
}

/// Equal seeds and operations reproduce equal delays within one build.
#[rstest]
fn equal_seeds_reproduce_equal_delays() {
    let make_policy = || {
        policy(
            Duration::from_millis(100),
            Duration::from_secs(10),
            Duration::from_secs(5),
        )
    };
    let mut first = BackoffState::with_seed(make_policy(), 1234);
    let mut second = BackoffState::with_seed(make_policy(), 1234);
    let start = Instant::now();
    let (mut first_delays, mut second_delays) = (Vec::new(), Vec::new());

    for step in 0..8_u64 {
        let now = start + Duration::from_millis(step * 100);
        first_delays.push(first.next_sleep(now));
        second_delays.push(second.next_sleep(now));
    }

    assert_eq!(first_delays, second_delays);
}

/// Repeating the initial failure timestamp must not double the bound.
///
/// With a base of 20 ms the first window samples `10..=20`. Any sample above
/// 20 ms would mean the delay doubled despite the timestamp never advancing.
#[rstest]
fn a_repeated_initial_timestamp_does_not_double() {
    let base = Duration::from_millis(20);
    let mut state = BackoffState::with_seed(
        policy(base, Duration::from_secs(60), Duration::from_secs(60)),
        1,
    );
    let start = Instant::now();

    for _ in 0..64 {
        let sleep = state.next_sleep(start).expect("within the deadline");
        assert!(
            sleep <= base,
            "a repeated initial timestamp widened the bound: {sleep:?}"
        );
    }
}

/// A later failure timestamp doubles the bound before sampling.
///
/// The first call at a base of 10 ms returns exactly 10 ms. The second call at
/// a later timestamp doubles the bound to 20 ms, so samples above 10 ms become
/// possible. Across 64 seeds the chance that every sample stays at the floor
/// is `(1/11)^64`, which is negligible, and the test is deterministic for the
/// chosen seeds within one build.
#[rstest]
fn a_distinct_timestamp_doubles_the_bound() {
    let policy = policy(
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_secs(60),
    );
    let start = Instant::now();
    let later = start + Duration::from_millis(1);
    let mut saw_above_minimum = false;

    for seed in 0..64 {
        let mut state = BackoffState::with_seed(policy.clone(), seed);
        assert_eq!(
            state.next_sleep(start),
            Some(Duration::from_millis(10)),
            "the first delay is the base bound"
        );
        let sleep = state.next_sleep(later).expect("within the deadline");
        assert!(
            (Duration::from_millis(10)..=Duration::from_millis(20)).contains(&sleep),
            "the doubled bound was not respected: {sleep:?}"
        );
        saw_above_minimum |= sleep > Duration::from_millis(10);
    }

    assert!(
        saw_above_minimum,
        "doubling must widen the bound above the minimum"
    );
}

// Cap and saturation contracts, split out to keep this file within the
// module-size limit.
#[path = "backoff_cap_tests.rs"]
mod cap;

// Deadline and reset contracts, split out for the same reason.
#[path = "backoff_reset_tests.rs"]
mod reset;
