//! Startup coordination for the process-wide dispatch pool.
//!
//! Keep the reservation and publication path separate from shard sizing and
//! dispatch operations. Besides making fork recovery easier to inspect, the
//! split keeps the parent dispatcher module within the repository's line limit.

use std::sync::atomic::Ordering;

use super::{Dispatcher, DispatcherError, POOL, POOL_STARTING_PROCESS, PoolStartGuard};

impl Dispatcher {
    /// Start the pool when this thread reserves it, or let the next loop try.
    pub(super) fn start_or_wait(process_id: u32) -> Result<(), DispatcherError> {
        match PoolStartGuard::reserve(&POOL_STARTING_PROCESS, process_id) {
            Ok(start_guard) => Self::start_reserved_pool(process_id, start_guard),
            Err(owner) => {
                Self::handle_start_reservation(process_id, owner);
                Ok(())
            }
        }
    }

    /// Start and publish a pool while keeping the reservation alive.
    fn start_reserved_pool(
        process_id: u32,
        _start_guard: PoolStartGuard<'_>,
    ) -> Result<(), DispatcherError> {
        // Recheck after reserving startup: another caller may have finished
        // between the shared lookup and this reservation.
        if Self::pool_for_process(process_id).is_none() {
            // Only the reservation holder starts workers, so concurrent first
            // use cannot exceed the fixed pool's thread bound.
            let candidate = Box::into_raw(Box::new(Self::start()?));
            POOL.store(candidate, Ordering::Release);
        }
        Ok(())
    }

    /// Wait for a local starter or clear a reservation inherited across fork.
    fn handle_start_reservation(process_id: u32, owner: u32) {
        if owner == process_id {
            std::thread::yield_now();
        } else {
            // A different PID can only be the parent reservation copied by
            // fork. The child is the sole survivor there, so it may safely
            // clear that stale reservation and retry.
            let _ = POOL_STARTING_PROCESS.compare_exchange(
                owner,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}
