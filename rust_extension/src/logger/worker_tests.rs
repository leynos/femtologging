//! Regression tests for dispatch state reserved across a fork rebind.

use std::time::Duration;

use super::{FemtoLogger, PidReservation, Shard};
use crate::logger::flush_clock::FlushDeadline;

#[test]
fn a_flush_reuses_state_published_before_its_reservation() {
    let shard = Box::leak(Box::new(
        Shard::start(0, 4).expect("isolated dispatch shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("fork-flush-race".to_owned(), shard);
    let process_id = std::process::id();
    let stale_process_id = process_id.wrapping_add(1);
    logger
        .dispatch
        .process_id
        .store(stale_process_id, std::sync::atomic::Ordering::Release);

    // This read represents the stale observation made by the caller that is
    // delayed before it can claim the reservation.
    assert_eq!(
        logger
            .dispatch
            .process_id
            .load(std::sync::atomic::Ordering::Acquire),
        stale_process_id
    );
    let winning_reservation =
        PidReservation::try_reserve(&logger.dispatch.starting_process, process_id)
            .expect("the first caller should claim the reservation");
    logger
        .dispatch
        .process_id
        .store(process_id, std::sync::atomic::Ordering::Release);
    drop(winning_reservation);

    let delayed_reservation =
        PidReservation::try_reserve(&logger.dispatch.starting_process, process_id)
            .expect("the delayed caller should claim the released reservation");
    let shard_before = logger
        .dispatch
        .shard
        .load(std::sync::atomic::Ordering::Acquire);
    let deadline = FlushDeadline::system(Duration::from_secs(1));

    let handles = logger
        .dispatch_state_reserved_until(process_id, &deadline)
        .expect("a published dispatch state should remain available")
        .expect("the deadline should remain available");

    assert!(
        std::ptr::eq(handles.0, shard_before),
        "the delayed caller must reuse the published shard instead of rebinding again"
    );
    drop(delayed_reservation);
}
