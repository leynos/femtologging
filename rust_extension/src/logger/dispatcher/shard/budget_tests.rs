//! Regression tests for logger-budget generation changes after `fork`.

use std::sync::Arc;

use super::{LoggerBudget, Permit, Permits};

// `LoggerBudget::new` and its limit accessor are `const`, which is what lets a
// budget be sized where it is declared. The wrapper below evaluates both in a
// const context, so losing either qualifier stops this file compiling.
// `limit_for_test` forwards to `limit`, so pinning the one pins both.
//
// A function-pointer binding would not do: it accepts a non-`const` `fn` and
// would pin the signature only. A `const` item holding a budget would not do
// either — the atomics are interior-mutable state, which clippy rightly
// refuses to see promoted to a constant.
const fn budget_limit(limit: usize) -> usize {
    LoggerBudget::new(limit).limit_for_test()
}
const _: fn(usize) -> usize = budget_limit;

#[test]
fn inherited_permit_cannot_release_a_child_budget_charge() {
    let permits = Arc::new(Permits::new(2));
    let budget = Arc::new(LoggerBudget::new(1));
    let inherited = Permit::take(&permits, &budget)
        .expect("the parent should charge its single allowed record");

    budget.reset_after_fork();
    let child = Permit::take(&permits, &budget).expect("the child should charge its fresh budget");

    drop(inherited);
    assert_eq!(
        budget.in_flight(),
        1,
        "the inherited permit must not release the child's charge"
    );

    drop(child);
    assert_eq!(budget.in_flight(), 0);
}

/// A release with no charge held saturates at zero instead of underflowing.
///
/// `release` runs from `Permit`'s drop, so it is reached on every path out of a
/// job, including the ones where a handler panics. A release with no matching
/// charge is therefore not hypothetical: a permit that was never charged, or
/// one released twice, both arrive here. Underflowing would wrap `in_flight` to
/// `usize::MAX`, and every subsequent comparison against the limit would refuse
/// records for the rest of the process's life.
///
/// The admission assertion is what makes this a behaviour test rather than an
/// arithmetic one: leaving the count at zero is only correct if the budget still
/// accepts its full limit afterwards.
#[test]
fn a_release_with_no_charge_held_saturates_at_zero() {
    let permits = Arc::new(Permits::new(2));
    let budget = Arc::new(LoggerBudget::new(2));
    let generation = budget.generation_for_test();

    for _ in 0..3 {
        budget.release(generation);
    }

    assert_eq!(
        budget.in_flight(),
        0,
        "a release with nothing in flight must leave the count at zero",
    );

    let first = Permit::take(&permits, &budget).expect("an empty budget admits its first record");
    let second = Permit::take(&permits, &budget).expect("an empty budget admits its second record");
    assert_eq!(budget.in_flight(), 2, "both charges must be held");
    assert!(
        Permit::take(&permits, &budget).is_none(),
        "the budget must still refuse past its limit after the spurious release",
    );

    drop(first);
    drop(second);
    assert_eq!(budget.in_flight(), 0, "both charges must be returned");
}
