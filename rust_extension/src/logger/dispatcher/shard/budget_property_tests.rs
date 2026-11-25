//! Property tests for the logger budget's accounting.
//!
//! The budget is the per-logger in-flight bound: a logger at its limit has its
//! records refused, and a permit's drop returns the charge. The example tests
//! pin the fork and zero-release cases; these check the arithmetic holds over
//! arbitrary sequences of charge, release and reset, which is where an
//! off-by-one or an underflow would hide.
//!
//! Every property is stated as an invariant that must hold after *each* step,
//! not only at the end: a transient overshoot is enough to refuse a record that
//! should have been admitted.

use std::sync::Arc;

use proptest::prelude::*;

use super::super::{LoggerBudget, Permit, Permits};

/// One operation against a budget under test.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Take a permit, keeping it alive.
    Take,
    /// Drop the oldest permit still held.
    Drop,
    /// Reset the in-flight count the way a post-fork shard does.
    Reset,
}

/// Generate a short operation sequence.
///
/// The length is bounded because each step may start nothing but the
/// interesting failures — overshoot and underflow — appear within a handful of
/// operations; long sequences only slow shrinking.
fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(
        prop_oneof![Just(Step::Take), Just(Step::Drop), Just(Step::Reset),],
        0..24,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// The in-flight count never exceeds the limit and never underflows.
    ///
    /// Both halves matter. An overshoot means the budget admitted a record past
    /// its bound, which is the starvation the budget exists to prevent. An
    /// underflow means a release had no charge to return and wrapped the count,
    /// after which every comparison against the limit refuses records forever.
    #[test]
    fn the_in_flight_count_stays_within_its_bounds(
        limit in 1usize..=4,
        steps in steps(),
    ) {
        let permits = Arc::new(Permits::new(8));
        let budget = Arc::new(LoggerBudget::new(limit));
        let mut held: Vec<Permit> = Vec::new();

        for step in steps {
            match step {
                Step::Take => {
                    if let Some(permit) = Permit::take(&permits, &budget) {
                        held.push(permit);
                    }
                }
                Step::Drop => {
                    if !held.is_empty() {
                        // Drop the oldest, so a stale charge is the one
                        // returned rather than always the newest.
                        drop(held.remove(0));
                    }
                }
                Step::Reset => budget.reset_after_fork(),
            }

            let in_flight = budget.in_flight();
            prop_assert!(
                in_flight <= limit,
                "in_flight {in_flight} exceeded the limit {limit} after {step:?}",
            );
        }

        // Draining everything must return the count to zero, whatever the
        // sequence did in between.
        held.clear();
        prop_assert_eq!(
            budget.in_flight(),
            0,
            "dropping every outstanding permit must return the count to zero",
        );
    }

    /// A budget admits exactly its limit and refuses the next record.
    ///
    /// Asserted from a fresh budget so the count is known: the boundary is what
    /// callers depend on, and an off-by-one would either strand a slot or admit
    /// one record too many.
    #[test]
    fn a_fresh_budget_admits_exactly_its_limit(limit in 1usize..=6) {
        let permits = Arc::new(Permits::new(limit + 2));
        let budget = Arc::new(LoggerBudget::new(limit));

        let mut held: Vec<Permit> = (0..limit)
            .map(|_| {
                Permit::take(&permits, &budget)
                    .expect("a fresh budget must admit its full limit")
            })
            .collect();

        prop_assert_eq!(budget.in_flight(), limit);
        prop_assert!(
            Permit::take(&permits, &budget).is_none(),
            "a budget at its limit must refuse the next record",
        );

        // Returning one charge must re-open exactly one slot.
        drop(held.pop());
        prop_assert_eq!(budget.in_flight(), limit - 1);
        prop_assert!(
            Permit::take(&permits, &budget).is_some(),
            "returning a charge must admit one more record",
        );
    }
}
