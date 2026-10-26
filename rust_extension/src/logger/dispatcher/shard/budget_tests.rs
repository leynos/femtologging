//! Regression tests for logger-budget generation changes after `fork`.

use std::sync::Arc;

use super::{LoggerBudget, Permit, Permits};

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
