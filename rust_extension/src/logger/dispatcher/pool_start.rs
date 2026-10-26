//! Startup coordination for the process-wide dispatch pool.
//!
//! Keep the reservation and publication path separate from shard sizing and
//! dispatch operations. Besides making fork recovery easier to inspect, the
//! split keeps the parent dispatcher module within the repository's line limit.

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::{Dispatcher, DispatcherError, POOL, POOL_STARTING_PROCESS};
use crate::logger::pid_reservation::PidReservation;

impl Dispatcher {
    /// Start the pool when this thread reserves it, or let the next loop try.
    pub(super) fn start_or_wait(process_id: u32) -> Result<(), DispatcherError> {
        match PidReservation::try_reserve(&POOL_STARTING_PROCESS, process_id) {
            Ok(start_guard) => Self::start_reserved_pool(process_id, start_guard),
            Err(owner) => {
                PidReservation::wait_or_clear_stale(&POOL_STARTING_PROCESS, process_id, owner);
                Ok(())
            }
        }
    }

    /// Start the pool or yield to its owner only while `deadline` remains.
    pub(super) fn start_or_wait_until(
        process_id: u32,
        deadline: Instant,
    ) -> Result<(), DispatcherError> {
        if Instant::now() > deadline {
            return Ok(());
        }
        match PidReservation::try_reserve(&POOL_STARTING_PROCESS, process_id) {
            Ok(start_guard) => Self::start_reserved_pool(process_id, start_guard),
            Err(owner) => {
                PidReservation::wait_or_clear_stale(&POOL_STARTING_PROCESS, process_id, owner);
                Ok(())
            }
        }
    }

    /// Start and publish a pool while keeping the reservation alive.
    fn start_reserved_pool(
        process_id: u32,
        start_guard: PidReservation<'_>,
    ) -> Result<(), DispatcherError> {
        Self::start_reserved_pool_with(process_id, start_guard, Self::start)
    }

    /// Start and publish a pool produced by `start_pool` while holding its reservation.
    pub(super) fn start_reserved_pool_with(
        process_id: u32,
        _start_guard: PidReservation<'_>,
        start_pool: impl FnOnce() -> Result<Self, DispatcherError>,
    ) -> Result<(), DispatcherError> {
        // Recheck after reserving startup: another caller may have finished
        // between the shared lookup and this reservation.
        if Self::pool_for_process(process_id).is_none() {
            // Only the reservation holder starts workers, so concurrent first
            // use cannot exceed the fixed pool's thread bound.
            let candidate = Box::into_raw(Box::new(start_pool()?));
            POOL.store(candidate, Ordering::Release);
        }
        Ok(())
    }
}
