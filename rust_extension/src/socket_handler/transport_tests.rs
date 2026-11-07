//! The deadline arithmetic a connection attempt is budgeted by.
//!
//! [`step_budget`] decides whether one step of a connect sequence may start and
//! how long it may take. It reads no clock and opens no socket: its answer is a
//! pure function of the signal's phase, the attempt's deadline, and the instant
//! the caller sampled. That is what lets the boundary cases below - a deadline
//! exactly reached, one nanosecond short, one long past - be asserted exactly
//! rather than raced against the host's scheduler.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::step_budget;
use crate::shutdown_signal::ShutdownSignal;

/// The budget a test connection attempt is allowed to spend.
const ATTEMPT: Duration = Duration::from_secs(1);

/// A running handler leaves the attempt's own deadline as the only ceiling.
///
/// Before shutdown begins the signal contributes nothing, so the budget is
/// exactly what the attempt has left. The attempt deadline is passed a full
/// second in the past here, which a running signal must not rescue: this is
/// the distinction between "the handler asked for nothing" and "the handler
/// granted the full configured timeout".
#[rstest]
fn a_running_handler_clamps_to_the_attempts_own_deadline() {
    let signal = ShutdownSignal::new();
    let now = Instant::now();
    let deadline = now - ATTEMPT;

    assert_eq!(step_budget(&signal, deadline, now), None);
}

/// A shutdown budget tighter than the attempt wins.
///
/// Both ceilings apply once a drain begins, and the smaller has to be the one
/// handed over. A helper that returned the attempt's remainder here would let
/// one connect step outlast the handler's deadline.
#[rstest]
fn a_shutdown_budget_tighter_than_the_attempt_wins() {
    let signal = ShutdownSignal::new();
    let now = Instant::now();
    signal.begin_shutdown(now + Duration::from_millis(20));

    let budget = step_budget(&signal, now + ATTEMPT, now)
        .expect("a live shutdown budget should allow the step");

    assert!(
        budget <= Duration::from_millis(20),
        "the shutdown budget must bound the step, got {budget:?}"
    );
}

/// A spent attempt deadline is `None` however healthy the shutdown budget is.
///
/// The attempt's deadline is the overall bound on the connect sequence, so an
/// expired one must stop the next step even while the handler is running and
/// would otherwise grant the full timeout.
#[rstest]
fn an_expired_attempt_deadline_stops_the_step() {
    let signal = ShutdownSignal::new();
    let now = Instant::now();

    assert_eq!(
        step_budget(&signal, now, now),
        None,
        "a deadline of exactly now leaves no time to start"
    );
}

/// A zero remainder is `None` rather than a zero timeout.
///
/// `std` and `ureq` both read a zero duration as "no timeout", so returning
/// one would ask for exactly the unbounded wait the budget is meant to forbid.
#[rstest]
fn a_zero_remainder_is_not_started() {
    let signal = ShutdownSignal::new();
    let now = Instant::now();

    assert_eq!(
        step_budget(
            &signal,
            now + Duration::from_nanos(1),
            now + Duration::from_nanos(1)
        ),
        None,
        "nothing may start with no time left, not even for a nanosecond"
    );
}
