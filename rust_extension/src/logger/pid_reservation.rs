//! Lock-free reservations for work that must happen once in each process.
//!
//! Pool startup and inherited-logger rebinding both need one caller to own a
//! reservation, release it after success or failure, and recover a reservation
//! copied from a parent process during `fork`.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::logger::flush_clock::FlushDeadline;

/// A process-local reservation released when its owner leaves scope.
pub(crate) struct PidReservation<'a> {
    /// Atomic word this reservation releases when it is dropped.
    reservation: &'a AtomicU32,
    /// Process ID that claimed the reservation.
    process_id: u32,
}

impl<'a> PidReservation<'a> {
    /// Claim a free reservation for `process_id`.
    pub(crate) fn try_reserve(reservation: &'a AtomicU32, process_id: u32) -> Result<Self, u32> {
        reservation
            .compare_exchange(0, process_id, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self {
                reservation,
                process_id,
            })
    }

    /// Wait for a local owner, or clear a reservation inherited across `fork`.
    pub(crate) fn wait_or_clear_stale(reservation: &AtomicU32, process_id: u32, owner: u32) {
        if owner == process_id {
            std::thread::yield_now();
        } else {
            // A different PID can only be the parent reservation copied by
            // fork. Its child may clear it because the parent thread is gone.
            let _ = reservation.compare_exchange(owner, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    /// Wait for a same-process owner only while the caller's deadline remains.
    pub(crate) fn wait_or_clear_stale_until(
        reservation: &AtomicU32,
        process_id: u32,
        owner: u32,
        deadline: &FlushDeadline,
    ) -> bool {
        if owner != process_id {
            let _ = reservation.compare_exchange(owner, 0, Ordering::AcqRel, Ordering::Acquire);
            return !deadline.is_expired();
        }

        while reservation.load(Ordering::Acquire) == process_id {
            if deadline.is_expired() {
                return false;
            }
            std::thread::yield_now();
        }
        !deadline.is_expired()
    }
}

impl Drop for PidReservation<'_> {
    fn drop(&mut self) {
        let _ = self.reservation.compare_exchange(
            self.process_id,
            0,
            Ordering::Release,
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
#[path = "pid_reservation_tests.rs"]
mod tests;
