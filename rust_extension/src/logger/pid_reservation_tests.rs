//! Tests for local reservation ownership and stale parent recovery.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::PidReservation;
use crate::logger::flush_clock::{FlushDeadline, MonotonicClock};

/// Clock that advances through predetermined reservation-wait samples.
struct SequenceClock {
    /// Samples consumed by the deadline checks.
    samples: Mutex<VecDeque<Instant>>,
    /// Time returned after all explicit samples are consumed.
    fallback: Instant,
}

impl MonotonicClock for SequenceClock {
    fn now(&self) -> Instant {
        self.samples
            .lock()
            .expect("reservation clock lock should not be poisoned")
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

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

#[test]
fn a_flush_stops_waiting_for_a_local_owner_at_its_deadline() {
    let reservation = AtomicU32::new(17);
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_millis(5);
    let clock = Arc::new(SequenceClock {
        samples: Mutex::new(VecDeque::from([started_at, started_at, deadline])),
        fallback: deadline,
    });
    let timing = FlushDeadline::with_clock(Duration::from_millis(5), clock);

    assert!(!PidReservation::wait_or_clear_stale_until(
        &reservation,
        17,
        17,
        &timing,
    ));
    assert_eq!(reservation.load(Ordering::Acquire), 17);
}
