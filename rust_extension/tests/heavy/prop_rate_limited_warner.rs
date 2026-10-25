//! Property-based tests for `RateLimitedWarner`'s drop accounting.
//!
//! The warner promises conservation: every recorded drop is reported exactly
//! once, whether it is reported by a due warning or by an explicit flush. That
//! promise is a statement about interleavings, so it is checked here against
//! generated ones rather than against a handful of scripted ones.
//!
//! Each case builds one shared warner over a public `Clock`, drives it from
//! several threads whose operations come from the same generator, and asserts
//! that reported plus pending equals recorded once every thread has finished
//! and a final flush has drained the remainder. The threads synchronize on a
//! `Barrier` so the race is real rather than merely available, and exactly one
//! thread moves the clock, so the interval boundary is a controlled input
//! rather than a function of how long the machine took.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use _femtologging_rs::rate_limited_warner::{Clock, RateLimitedWarner};
use proptest::prelude::*;

/// A public [`Clock`] whose reading is driven by the test.
///
/// [`RateLimitedWarner::with_clock`] takes an `Arc<dyn Clock>`, so this is the
/// seam a property test uses to make "has the interval elapsed?" a decision the
/// case makes rather than one the scheduler makes.
struct StepClock {
    /// Current fake time in milliseconds.
    now_ms: AtomicU64,
}

impl Clock for StepClock {
    fn now_millis(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }
}

impl StepClock {
    fn new() -> Self {
        Self {
            now_ms: AtomicU64::new(0),
        }
    }

    fn advance(&self, milliseconds: u64) {
        self.now_ms.fetch_add(milliseconds, Ordering::Relaxed);
    }
}

/// One operation a worker thread may perform.
#[derive(Debug, Clone)]
enum Op {
    /// Record a drop, as a handler does when it refuses a record.
    Record,
    /// Offer to warn; a suppressed call must leave the counter alone.
    WarnIfDue,
    /// Report everything pending, regardless of the interval.
    Flush,
    /// Move the clock forward, crossing the interval boundary.
    Advance(u64),
}

impl Op {
    fn is_advance(&self) -> bool {
        matches!(self, Self::Advance(_))
    }
}

/// A generated case: one operation sequence per thread, each non-empty, with
/// the clock-owning thread's sequence first.
type Case = Vec<Vec<Op>>;

/// Operations for the threads that do not drive the clock.
///
/// `Advance` is omitted rather than generated and filtered out, so no case can
/// move the clock from two threads at once. That would make the interval
/// boundary unpredictable without exercising anything conservation depends on.
fn non_clock_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => Just(Op::Record),
        5 => Just(Op::WarnIfDue),
        2 => Just(Op::Flush),
    ]
}

/// Operations for the single thread that owns the clock.
fn clock_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => Just(Op::Record),
        5 => Just(Op::WarnIfDue),
        2 => Just(Op::Flush),
        4 => (1u64..=5000).prop_map(Op::Advance),
    ]
}

/// Ensure a sequence crosses an interval boundary, so the run cannot be a
/// sequence of suppressions that never reports.
///
/// The generator is random, so "an advance happens to appear" is not something
/// to rely on. Inserting one when the draw omitted it keeps every case inside
/// the state space the property is about, instead of filtering draws out.
fn with_advance(mut ops: Vec<Op>) -> Vec<Op> {
    if !ops.iter().any(Op::is_advance) {
        ops.insert(0, Op::Advance(1));
    }
    ops
}

/// Build a case: a clock-owning sequence followed by one to three others.
fn case() -> impl Strategy<Value = Case> {
    // A handful of short sequences, so a case is tens to low hundreds of
    // operations. The property is about interleaving, not volume.
    (
        proptest::collection::vec(clock_op(), 10..50),
        proptest::collection::vec(proptest::collection::vec(non_clock_op(), 1..30), 1..3),
    )
        .prop_map(|(clock_ops, others)| {
            let mut threads = vec![with_advance(clock_ops)];
            threads.extend(others);
            // Ensure at least one drop in the case overall, so conservation is
            // never asserted over a run that had nothing to conserve.
            if !threads.iter().flatten().any(|op| matches!(op, Op::Record)) {
                threads[0].insert(0, Op::Record);
            }
            threads
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        max_shrink_iters: 2000,
        ..ProptestConfig::default()
    })]

    /// Conservation under generated interleavings.
    ///
    /// Every `Record` operation increments the counter exactly once and every
    /// drain reports what it removed, so the reported total plus whatever is
    /// still pending must equal the number of `Record` operations. The final
    /// flush is what makes this sharp: a drop no warning ever reported is still
    /// pending, so a warner that discarded a suppressed batch fails here even
    /// though the run "warned" successfully.
    ///
    /// Runs only in the nightly heavy job: each case spawns threads, which is
    /// too slow for the ordinary gate.
    #[test]
    #[ignore = "spawns threads per generated case; too slow for the ordinary gate"]
    fn prop_rate_limited_warner_conserves_drops(case in case()) {
        // The generator is built so these always hold; asserting them keeps the
        // invariants below from resting on an unstated assumption, and turns a
        // generator mistake into a failure rather than into a vacuous pass.
        prop_assert!(!case.is_empty(), "a case must have at least one thread");
        prop_assert!(
            case[0].iter().any(Op::is_advance),
            "the clock-owning thread must cross the interval boundary"
        );
        for ops in case.iter().skip(1) {
            prop_assert!(
                !ops.iter().any(Op::is_advance),
                "only the first thread may advance the clock"
            );
        }

        let expected_records: u64 = case
            .iter()
            .flatten()
            .filter(|op| matches!(op, Op::Record))
            .count() as u64;
        // A case that recorded nothing would make the equality below trivially
        // true, so it is a generator failure rather than evidence.
        prop_assert!(
            expected_records > 0,
            "the generator produced a case with no drop to conserve"
        );

        let clock = Arc::new(StepClock::new());
        // A short interval keeps the boundary reachable: advances are tens of
        // milliseconds, so most of them cross it and the run exercises the due
        // and suppressed paths alike.
        let warner = Arc::new(RateLimitedWarner::with_clock(
            Duration::from_millis(10),
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));

        let recorded = AtomicU64::new(0);
        let reported = AtomicU64::new(0);
        let start = Barrier::new(case.len());

        let () = thread::scope(|scope| {
            for ops in case.iter() {
                let warner = Arc::clone(&warner);
                let clock = Arc::clone(&clock);
                let recorded = &recorded;
                let reported = &reported;
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    let mut warn = |count: u64| {
                        reported.fetch_add(count, Ordering::Relaxed);
                    };
                    for op in ops {
                        match op {
                            // Recording and the reference count are incremented
                            // separately so they can only diverge if the helper
                            // misses an increment, which is what the assertion
                            // tests.
                            Op::Record => {
                                warner.record_drop();
                                recorded.fetch_add(1, Ordering::Relaxed);
                            }
                            Op::WarnIfDue => warner.warn_if_due(&mut warn),
                            Op::Flush => warner.flush(&mut warn),
                            // Only the first thread is given `Advance`, which
                            // the assertions above establish, so this cannot
                            // race with another advance.
                            Op::Advance(ms) => clock.advance(*ms),
                        }
                    }
                });
            }
        });

        let mut tail = Vec::new();
        warner.flush(|count| tail.push(count));
        let reported = reported.load(Ordering::Relaxed) + tail.iter().sum::<u64>();
        let pending = warner.pending_drops();
        let recorded = recorded.load(Ordering::Relaxed);

        prop_assert_eq!(
            recorded, expected_records,
            "every Record operation should have reached the warner"
        );
        prop_assert_eq!(
            pending, 0,
            "the final flush should leave nothing pending"
        );
        prop_assert_eq!(
            reported, recorded,
            "reported drops plus pending drops should equal recorded drops"
        );
    }
}
