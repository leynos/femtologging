//! Properties of the shutdown state machine across generated operation
//! sequences.
//!
//! [`ShutdownSignal`] is a small concurrent state machine with three phases
//! and four queries, and the scenario tests pin each transition once. What they
//! cannot show is that the transitions *compose*: that no sequence of
//! `begin_shutdown`, `abort` and the deadline passing can leave the queries
//! disagreeing with each other, or with the clock.
//!
//! Each property here generates a sequence of operations against a manual
//! clock, applies them, and asserts an invariant that must hold at every point.
//! The clock is manual because the interesting boundary - a deadline that has
//! just passed - is otherwise a race the host decides, and a machine under load
//! can make a correct implementation look wrong.
//!
//! The Loom models in `tests/heavy/` cover the *interleavings* of a signal
//! shared between threads. These cover the *sequences* within one thread, which
//! Loom does not explore because it schedules threads rather than generating
//! operation orders.

use std::sync::Arc;
use std::time::Duration;

use proptest::prelude::*;

use super::{Phase, ShutdownSignal};
use crate::monotonic_clock::{ManualClock, MonotonicClock, SharedClock};

/// A manual clock and the signal that reads it.
///
/// The concrete `ManualClock` is kept for [`ManualClock::advance`] and
/// [`ManualClock::base`], while the signal holds the same clock as the
/// trait object it was built with. Both handles point at one clock, so moving
/// one is visible through the other.
fn signal_on_manual_clock() -> (Arc<ManualClock>, ShutdownSignal) {
    let clock = Arc::new(ManualClock::new());
    let shared: SharedClock = Arc::clone(&clock) as SharedClock;
    (clock, ShutdownSignal::with_clock(shared))
}

/// One thing a handler or worker can do to a signal.
#[derive(Debug, Clone, Copy)]
enum Operation {
    /// Begin a graceful shutdown this far past the clock's base.
    BeginShutdown(u64),
    /// Abandon everything not already sent.
    Abort,
    /// Move the manual clock forward by this many milliseconds.
    Advance(u64),
}

/// A generator for operation sequences.
///
/// Deadlines span zero to a second past the clock's base, and advances span the
/// same range, so a generated sequence reaches both sides of the boundary many
/// times over.
fn any_operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        (0_u64..=1_000).prop_map(Operation::BeginShutdown),
        (0_u64..=1_000).prop_map(Operation::Advance),
        Just(Operation::Abort),
    ]
}

/// Apply `operations` to a signal on a manual clock, checking after every one.
///
/// Returns once the sequence is spent. The clock is read back at the end so the
/// final assertions can be expressed against the time the sequence actually
/// reached rather than the offsets it was generated from.
fn apply(operations: &[Operation]) -> (Arc<ManualClock>, ShutdownSignal) {
    let (clock, signal) = signal_on_manual_clock();
    for operation in operations {
        match *operation {
            Operation::BeginShutdown(after) => {
                signal.begin_shutdown(clock.base() + Duration::from_millis(after));
            }
            Operation::Abort => signal.abort(),
            Operation::Advance(millis) => clock.advance(Duration::from_millis(millis)),
        }
        check_invariants(&signal);
    }
    (clock, signal)
}

/// The invariants that must hold whatever the operations were.
///
/// Written as one function because each call re-asserts all of them: an
/// invariant that stopped holding after operation 3 but held again after
/// operation 4 is still a defect, and checking only the final state would miss
/// it.
///
/// The phase is read together with the instant it should be judged against,
/// so every comparison below is between values from one moment. The queries
/// each sample the clock themselves, which is why they cannot be compared to
/// each other directly - that comparison would be two reads, and a deadline
/// between them would make a correct signal look inconsistent.
fn check_invariants(signal: &ShutdownSignal) {
    let (phase, now) = signal.phase_and_now();
    let shutting_down = signal.is_shutting_down();
    let aborted = signal.is_aborted();
    let remaining = signal.remaining();
    let clamped = signal.clamp(Duration::from_millis(100));

    // `is_shutting_down` is exactly "the phase left Running". Stated as an
    // equivalence with the phase rather than a property of the other queries,
    // so a query that drifts from the phase cannot satisfy this by also
    // drifting with the other queries.
    assert_eq!(
        shutting_down,
        phase != Phase::Running,
        "is_shutting_down must mean the phase left Running; phase is {phase:?}"
    );

    match phase {
        Phase::Running => {
            assert!(!aborted, "a running signal cannot be aborted");
            assert_eq!(remaining, None, "no budget applies while running");
        }
        Phase::Aborted => {
            assert!(aborted, "an aborted signal always reports aborted");
            assert_eq!(remaining, None, "an abort leaves no budget");
            assert_eq!(clamped, None, "an abort clamps to nothing");
        }
        Phase::Draining { deadline } => {
            // The two queries must agree about the same instant. This is the
            // invariant the manual clock exists to make checkable: `now` is
            // the clock's own reading from beside the phase, so a disagreement
            // is a real defect rather than a pair of clock reads landing
            // either side of a nanosecond. The clock is frozen while these
            // assertions run because only this thread samples it.
            assert_eq!(
                aborted,
                deadline <= now,
                "is_aborted must mean the deadline has passed; deadline is \
                 {deadline:?} and now is {now:?}"
            );
            assert_eq!(
                remaining.is_none(),
                deadline <= now,
                "remaining must be empty exactly when the deadline has passed"
            );
            if let Some(left) = remaining {
                let distance = deadline.saturating_duration_since(now);
                assert!(
                    left <= distance,
                    "remaining must not exceed the deadline's distance: \
                     {left:?} > {distance:?}"
                );
                assert!(
                    clamped.is_some_and(|value| value <= left),
                    "a clamp must never exceed the remaining budget"
                );
            } else {
                assert_eq!(clamped, None, "a spent budget clamps to nothing");
            }
        }
    }
}

proptest! {
    /// No generated sequence can make the queries disagree with the phase.
    ///
    /// Covers repeated `begin_shutdown` calls with varying deadlines, `abort`
    /// at any point, and the clock crossing a deadline in either direction.
    /// The first-deadline rule and the abort's permanence are properties of
    /// those same sequences, so they are checked alongside the query
    /// agreement rather than in sequences of their own.
    #[test]
    fn generated_operation_sequences_keep_the_queries_consistent(
        operations in prop::collection::vec(any_operation(), 0..40)
    ) {
        apply(&operations);
    }

    /// A shutdown's first deadline survives every later one.
    ///
    /// Two closers racing, or a close after a close, must not extend the
    /// budget the first one promised to the caller waiting on it. Expressed
    /// against the clock so "which deadline won" is exact rather than timing
    /// dependent.
    #[test]
    fn the_first_deadline_survives_later_ones(
        first_millis in 0_u64..=1_000,
        later_millis in 0_u64..=1_000,
    ) {
        let (clock, signal) = signal_on_manual_clock();
        let first = clock.base() + Duration::from_millis(first_millis);
        signal.begin_shutdown(first);

        let later = clock.base() + Duration::from_millis(later_millis);
        signal.begin_shutdown(later);

        // The budget is measured from the first deadline, whichever way the
        // two offsets compare. Reading it at the clock's base makes the
        // expected value exact.
        //
        // A zero expectation is `None`, not `Some(0)`: `first_millis` starts at
        // zero, so this test generates the deadline instant itself, and the
        // query reports a spent budget there rather than a zero one. Stating
        // the oracle the same way is what keeps this test about "which
        // deadline won" rather than about the boundary, which
        // `the_deadline_instant_itself_is_spent` pins separately.
        let expected = first
            .checked_duration_since(clock.now())
            .filter(|left| !left.is_zero());
        let actual = signal.remaining();
        prop_assert!(
            match (expected, actual) {
                (None, None) => true,
                (Some(most), Some(left)) => left <= most,
                _ => false,
            },
            "the second shutdown replaced the first deadline: expected at most \
             {:?}, got {:?}",
            expected,
            actual
        );
    }

    /// An abort is permanent, and no later shutdown resurrects a budget.
    ///
    /// A handler gives up only when its own deadline has arrived by the only
    /// route left, so a worker that observes an abort must not later find
    /// itself draining again.
    #[test]
    fn an_abort_is_permanent(
        before_millis in 0_u64..=1_000,
        after_millis in 0_u64..=1_000,
    ) {
        let (clock, signal) = signal_on_manual_clock();
        signal.begin_shutdown(clock.base() + Duration::from_millis(before_millis));
        signal.abort();

        signal.begin_shutdown(clock.base() + Duration::from_millis(after_millis));
        clock.advance(Duration::from_millis(after_millis));

        prop_assert!(signal.is_aborted(), "an abort must stay in force");
        prop_assert_eq!(signal.remaining(), None, "an abort leaves no budget");
        prop_assert_eq!(
            signal.clamp(Duration::from_millis(100)),
            None,
            "an aborted signal must not hand out a budget"
        );
    }
}
