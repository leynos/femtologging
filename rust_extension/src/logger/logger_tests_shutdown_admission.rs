//! Admission-bound tests for shared logger dispatch shards.

use super::*;

/// One logger's budget refuses another record when its own limit is reached.
#[test]
fn one_shard_refuses_records_beyond_the_logger_budget() {
    // A one-deep shard gives its logger a one-record budget. The held handler
    // keeps that record in flight so the second call reaches the same bound.
    let shard = Shard::start(0, 1).expect("a test shard should start");
    let budget = shard.budget();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let handler = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    }) as Arc<dyn FemtoHandlerTrait>;

    assert!(
        shard
            .admit(queued_record("bounded", "held", &handler), &budget)
            .is_ok(),
        "the first record fits"
    );
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should start handling the first record");
    let refused = shard.admit(queued_record("bounded", "refused", &handler), &budget);

    release_tx
        .send(())
        .expect("the worker should still be waiting");
    assert!(
        shard.wait_for_drain(),
        "the worker should drain after release"
    );
    let refused = refused.expect_err("the logger's full budget must refuse another record");
    assert_eq!(refused.record.message(), "refused");
}

/// A logger at its own limit leaves capacity for another logger on a deep shard.
#[test]
fn one_stuck_logger_does_not_take_another_logger_s_slot() {
    let shard = Shard::start(0, 2).expect("a test shard should start");
    let stuck_budget = shard.budget_with_limit_for_test(1);
    let other_budget = shard.budget();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let stuck_handler = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    }) as Arc<dyn FemtoHandlerTrait>;

    assert!(
        shard
            .admit(
                queued_record("stuck", "first", &stuck_handler),
                &stuck_budget
            )
            .is_ok(),
        "the first record fits"
    );
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should start handling the first record");
    assert!(
        shard
            .admit(
                queued_record("stuck", "second", &stuck_handler),
                &stuck_budget
            )
            .is_err(),
        "a logger at its own in-flight allowance must be refused"
    );

    let collector = Arc::new(CollectingHandler::new());
    let other_handler = Arc::clone(&collector) as Arc<dyn FemtoHandlerTrait>;
    assert!(
        shard
            .admit(
                queued_record("other", "delivered", &other_handler),
                &other_budget,
            )
            .is_ok(),
        "the other logger must have its own allowance"
    );

    release_tx
        .send(())
        .expect("the worker should still be waiting for its first record");
    assert!(
        shard.wait_for_drain(),
        "the shard should drain once the stuck handler returns"
    );
    assert_eq!(
        collected_messages(&collector),
        vec!["delivered"],
        "the other logger's record should be delivered after the handler returns"
    );
}

/// A failed shard permit acquisition returns the charge to that logger.
#[test]
fn shard_permit_exhaustion_rolls_back_the_logger_charge() {
    let shard = Shard::start(0, 1).expect("a test shard should start");
    let occupying_budget = shard.budget_with_limit_for_test(1);
    let refused_budget = shard.budget_with_limit_for_test(1);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let handler = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    }) as Arc<dyn FemtoHandlerTrait>;

    assert!(
        shard
            .admit(
                queued_record("occupying", "held", &handler),
                &occupying_budget
            )
            .is_ok(),
        "the first logger should take the shard's only permit"
    );
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the worker should hold the first permit in its handler");
    let refusal = shard.admit(
        queued_record("refused", "not queued", &handler),
        &refused_budget,
    );
    let charge_after_refusal = refused_budget.in_flight();

    release_tx
        .send(())
        .expect("the worker should still be waiting");
    assert!(
        shard.wait_for_drain(),
        "the worker should drain after release"
    );
    assert!(
        refusal.is_err(),
        "the shard-wide permit pool must refuse work after its only permit is held"
    );
    assert_eq!(
        charge_after_refusal, 0,
        "a shard-wide refusal must return the second logger's budget charge"
    );
}
