//! Contracts for the shutdown signal's phases, budget and interruptible wait.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::{Phase, ShutdownSignal, WaitOutcome};

/// A delay long enough that only an early wakeup can beat it, and short enough
/// that a signal which fails to interrupt costs the suite a moment rather than
/// a minute.
const LONG_DELAY: Duration = Duration::from_secs(30);

/// The budget every drain in these tests is given.
const BUDGET: Duration = Duration::from_millis(500);

/// How long a test waits for another thread to reach a state it is trying to
/// arrange. Far longer than the work takes, so only a real failure exhausts it.
const SETTLE: Duration = Duration::from_secs(5);

fn draining() -> ShutdownSignal {
    let signal = ShutdownSignal::new();
    signal.begin_shutdown(Instant::now() + BUDGET);
    signal
}

/// Scenario: a signal nobody has touched.
///
/// Invariant: it reports ordinary operation, no budget, and a clamp that is
/// exactly what the caller configured. The last is the one that matters: a
/// running worker must reach the network with its configured timeout, not one
/// silently shortened by a mechanism that is not in use.
#[rstest]
fn a_fresh_signal_is_running() {
    let signal = ShutdownSignal::new();

    assert!(!signal.is_shutting_down(), "nothing has been asked of it");
    assert!(!signal.is_aborted(), "a running worker may keep working");
    assert_eq!(signal.remaining(), None, "no budget applies while running");
    assert_eq!(
        signal.clamp(BUDGET),
        Some(BUDGET),
        "clamp must return the configured timeout unchanged while running"
    );
}

/// Scenario: `wait` is called with nothing to interrupt it.
///
/// Invariant: it returns `Elapsed`, having actually slept. The elapsed-time
/// assertion is what separates a real sleep from a wait that returns at once;
/// without it a `wait` that ignored its delay entirely would pass every other
/// test here.
#[rstest]
fn wait_elapses_when_nothing_interrupts_it() {
    let signal = ShutdownSignal::new();
    let delay = Duration::from_millis(50);

    let started = Instant::now();
    let outcome = signal.wait(delay);

    assert_eq!(outcome, WaitOutcome::Elapsed);
    assert!(
        started.elapsed() >= delay,
        "wait returned after {:?}, before its {delay:?} delay",
        started.elapsed()
    );
}

/// Scenario: a graceful shutdown begins while a worker is sleeping.
///
/// Invariant: the worker wakes at once and is told to consider stopping, rather
/// than sleeping out the rest of its delay. This is the whole point of the
/// module: the delay a worker is in the middle of is the delay a close cannot
/// wait for.
#[rstest]
fn a_shutdown_wakes_a_waiting_worker() {
    let signal = Arc::new(ShutdownSignal::new());
    let waits = signal.notify_on_wait();
    let waiting = signal.clone();
    let sleeping = thread::spawn(move || waiting.wait(LONG_DELAY));
    waits
        .recv_timeout(SETTLE)
        .expect("the worker should reach its wait");

    let started = Instant::now();
    signal.begin_shutdown(Instant::now() + BUDGET);
    let outcome = sleeping.join().expect("the worker should not panic");

    assert_eq!(outcome, WaitOutcome::ShutdownRequested);
    assert!(
        started.elapsed() < BUDGET,
        "the shutdown took {:?} to reach the worker, which is not an interruption",
        started.elapsed()
    );
}

/// Scenario: `abort` lands while a worker is sleeping.
///
/// Invariant: the worker wakes and is told to stop, not merely to consider it.
/// Abort is the handler's decision that the budget is gone, so a worker that
/// treated it as an invitation to retry would still be running past the
/// deadline `close` promised to return by.
#[rstest]
fn an_abort_wakes_a_waiting_worker_told_to_stop() {
    let signal = Arc::new(ShutdownSignal::new());
    let waits = signal.notify_on_wait();
    let waiting = signal.clone();
    let sleeping = thread::spawn(move || waiting.wait(LONG_DELAY));
    waits
        .recv_timeout(SETTLE)
        .expect("the worker should reach its wait");

    signal.abort();

    assert_eq!(
        sleeping.join().expect("the worker should not panic"),
        WaitOutcome::Aborted
    );
}

/// Scenario: a worker is asleep and the drain deadline passes without anyone
/// calling `abort`.
///
/// Invariant: the wait reports `Aborted`. Nothing else will wake the worker:
/// the handler is waiting on an acknowledgement, not sending one, so a wait
/// that slept past its own budget and then reported `Elapsed` would let a
/// record be attempted after the deadline had passed.
#[rstest]
fn a_wait_started_after_the_deadline_reports_aborted() {
    let signal = ShutdownSignal::new();
    signal.begin_shutdown(Instant::now() + Duration::from_millis(1));
    thread::sleep(Duration::from_millis(20));

    assert_eq!(signal.wait(LONG_DELAY), WaitOutcome::Aborted);
}

/// Scenario: a wait begins after a graceful shutdown has already started.
///
/// Invariant: it reports the existing shutdown immediately instead of
/// sleeping. Workers must not retry after a shutdown begins, and the drain
/// path has no retry sleeps to wait out.
#[rstest]
fn a_wait_started_during_shutdown_reports_it_without_sleeping() {
    let signal = Arc::new(ShutdownSignal::new());
    let waits = signal.notify_on_wait();
    let waiting = signal.clone();
    let sleeping = thread::spawn(move || waiting.wait(LONG_DELAY));
    waits
        .recv_timeout(SETTLE)
        .expect("the worker should reach its wait");
    signal.begin_shutdown(Instant::now() + BUDGET);
    assert_eq!(
        sleeping.join().expect("the worker should not panic"),
        WaitOutcome::ShutdownRequested
    );

    assert!(!signal.is_aborted(), "the budget has barely been touched");

    let started = Instant::now();
    let outcome = signal.wait(Duration::from_secs(2));
    assert!(
        outcome == WaitOutcome::ShutdownRequested,
        "a wait during graceful shutdown must report the phase, got {outcome:?}"
    );
    assert!(
        started.elapsed() < BUDGET,
        "a wait during graceful shutdown must not sleep: it returned after {:?}",
        started.elapsed()
    );
}

/// Scenario: work is about to begin against a drain budget part-spent.
///
/// Invariant: `clamp` returns the smaller of the configured timeout and what
/// is left, so no single operation can run past the deadline the handler is
/// holding the worker to.
#[rstest]
fn clamp_is_bounded_by_the_remaining_budget() {
    let signal = ShutdownSignal::new();
    let configured = Duration::from_secs(30);
    signal.begin_shutdown(Instant::now() + BUDGET);

    let clamped = signal
        .clamp(configured)
        .expect("the budget is not yet spent");

    assert!(
        clamped <= BUDGET,
        "a {configured:?} timeout escaped a {BUDGET:?} budget as {clamped:?}"
    );
    assert!(
        clamped > Duration::ZERO,
        "a live budget must leave room to make one attempt"
    );
}

/// Scenario: a caller whose configured timeout is already shorter than the
/// budget.
///
/// Invariant: its own timeout wins. Clamping exists to shorten, not to lengthen,
/// and a drain that handed a worker more time than it was configured for would
/// be a shutdown raising its own limits.
#[rstest]
fn clamp_keeps_a_timeout_shorter_than_the_budget() {
    let signal = draining();
    let configured = Duration::from_millis(1);

    assert_eq!(signal.clamp(configured), Some(configured));
}

/// Scenario: a spent budget, and an aborted signal.
///
/// Invariant: both clamp to `None`, and both report `is_aborted`. `None` is the
/// instruction not to begin: `std` and `ureq` read a zero duration as "wait
/// forever", so a spent budget handed to either as zero would ask for the
/// opposite of what it means.
#[rstest]
#[case::expired_budget(false)]
#[case::explicit_abort(true)]
fn a_spent_budget_clamps_to_nothing(#[case] is_explicit_abort: bool) {
    let signal = ShutdownSignal::new();
    if is_explicit_abort {
        signal.begin_shutdown(Instant::now() + BUDGET);
        signal.abort();
    } else {
        signal.begin_shutdown(Instant::now() + Duration::from_millis(1));
        thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(signal.clamp(BUDGET), None);
    assert!(signal.is_aborted());
    assert!(signal.is_shutting_down(), "aborted is still shutting down");
}

/// Scenario: `abort` arrives after a drain has begun.
///
/// Invariant: it takes effect, and the deadline it replaces stops mattering.
/// The handler aborts precisely because its acknowledgement deadline fell
/// first, so a signal that held on to the original budget would keep telling
/// workers they had time they no longer had.
#[rstest]
fn abort_supersedes_an_earlier_deadline() {
    let signal = ShutdownSignal::new();
    signal.begin_shutdown(Instant::now() + LONG_DELAY);
    assert_eq!(
        signal.remaining().map(|left| left > BUDGET),
        Some(true),
        "the long budget should still be intact"
    );

    signal.abort();

    assert_eq!(signal.remaining(), None, "an abort leaves no budget");
    assert_eq!(signal.clamp(BUDGET), None);
}

/// Scenario: a second `begin_shutdown` arrives with a different deadline.
///
/// Invariant: the first deadline stands. Two closers racing, or a close after
/// a close, must not extend the budget the first one promised to the caller
/// who is waiting on it.
#[rstest]
fn a_second_shutdown_keeps_the_first_deadline() {
    let signal = ShutdownSignal::new();
    let first = Instant::now() + Duration::from_millis(50);
    signal.begin_shutdown(first);

    signal.begin_shutdown(Instant::now() + LONG_DELAY);

    let remaining = signal
        .remaining()
        .expect("the first budget should still be running");
    assert!(
        remaining <= Duration::from_millis(50),
        "the later shutdown extended the budget to {remaining:?}"
    );
}

/// Scenario: the phase is inspected from the debugger.
///
/// Invariant: a `Debug` rendering names the phase rather than panicking on the
/// lock. These types are read from test failures and log output.
#[rstest]
fn debug_reports_the_phase() {
    let signal = draining();

    let rendered = format!("{signal:?}");
    assert!(
        rendered.contains("Draining"),
        "a draining signal should say so: {rendered}"
    );
    assert_eq!(Phase::Running, Phase::Running);
}
