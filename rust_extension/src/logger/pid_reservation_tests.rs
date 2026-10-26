//! Tests for local reservation ownership and stale parent recovery.

use std::sync::atomic::{AtomicU32, Ordering};

use super::PidReservation;

#[test]
fn a_local_owner_keeps_its_reservation() {
    let reservation = AtomicU32::new(17);

    PidReservation::wait_or_clear_stale(&reservation, 17, 17);

    assert_eq!(reservation.load(Ordering::Acquire), 17);
}

#[test]
fn a_fork_child_clears_its_parent_reservation() {
    let reservation = AtomicU32::new(17);

    PidReservation::wait_or_clear_stale(&reservation, 29, 17);

    assert_eq!(reservation.load(Ordering::Acquire), 0);
}
