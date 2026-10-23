//! Background dispatch of queued log records.
//!
//! One logger used to mean one worker thread, so a process holding a logger per
//! component held a thread per component. Threads are the expensive half of
//! that: each carries a stack, a kernel scheduling object and a slot against
//! the process's thread limit, and a process that registers loggers from
//! caller-supplied names runs out of them long before it runs out of memory.
//!
//! A dispatcher owns a small, fixed pool of *shards* instead, and every logger
//! is assigned one. Threads spent on logging are then a function of the
//! machine, not of how many loggers exist, and the pool is shared by every
//! logger in the process.
//!
//! What the pool must not change:
//!
//! - **Per-logger FIFO order.** Every record a logger dispatches goes to that
//!   logger's shard, and a shard runs its queue in order, so one logger's
//!   records are handled in the order they were logged. Two loggers sharing a
//!   shard have no order relative to each other, which was never promised.
//! - **Flush before returning.** A flush acknowledges through the same queue,
//!   so an acknowledgement means everything queued ahead of it was handled.
//! - **Draining on drop.** A logger's `Drop` waits for its shard to drain what
//!   is already queued, so no record is lost on the way out.
//! - **Isolation from a panicking handler.** A shard catches a panic, reports
//!   it and continues, so one logger's handler cannot silence the loggers
//!   sharing the shard.
//!
//! Nothing here blocks the logging call: a full shard returns the record to be
//! counted as dropped, exactly as a full per-logger queue did, so a slow
//! handler degrades the loggers on its shard rather than the process.
//!
//! The pool is process-wide, started once on first use, and deliberately never
//! torn down: a process that has finished with the loggers it created will make
//! more, and reclaiming a handful of idle threads would buy nothing.
//! Starting it can fail, because a process at its thread limit cannot have
//! them; that failure is returned to the caller rather than panicking, and a
//! later logger retries.

mod shard;

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod dispatcher_tests;

#[cfg(not(loom))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(not(loom))]
use once_cell::sync::{Lazy, OnceCell};

pub(crate) use shard::{LoggerBudget, Shard};

/// Threads a dispatcher starts when the machine reports no useful limit.
///
/// Four shards are enough that one logger with a slow handler does not hold up
/// every other logger in the process, and few enough that the threads are not
/// themselves the problem.
const DEFAULT_SHARD_COUNT: usize = 4;

/// Shallowest pool a machine's available parallelism may produce.
const MIN_SHARD_COUNT: usize = 1;

/// Records one shard will hold in flight per point of shard count.
///
/// The depth is multiplied by the shard count, so the pool holds as much queued
/// work in total as the per-logger queues it replaced.
///
/// It doubles as one logger's in-flight allowance on a shard, which is the
/// bound that makes sharing safe: the shard-wide pool alone bounds what the
/// shard holds, not what one logger takes from it, so a single logger whose
/// handler never returns could otherwise consume every permit. Both uses are
/// this constant so the total the pool holds and the share one logger may take
/// are stated in one place.
pub(crate) const RECORDS_PER_LOGGER: usize = 1024;

/// The depth a Loom model's shard is built with.
///
/// A real shard's depth is [`RECORDS_PER_LOGGER`], and filling its permit pool
/// costs one channel send per slot. Under Loom every send is a scheduling
/// point, so a 1024-deep pool spends the whole branch budget before the model
/// has exercised any behaviour — the topology models then fail with "exceeded
/// maximum number of branches" rather than on a property.
///
/// The models assert which handlers a record reaches and how often, and that
/// needs three records in flight at most. Eight is comfortably above that and
/// small enough that the branch space stays explorable.
#[cfg(loom)]
pub(crate) const LOOM_RECORDS_PER_LOGGER: usize = 8;

/// A running pool of dispatch shards.
///
/// Absent under `--cfg loom`, where a model owns its one shard instead; see
/// [`POOL`]. Gated rather than left unused so a Loom build, which CI lints with
/// `-D warnings`, does not fail on dead code it structurally cannot reach.
#[cfg(not(loom))]
pub(crate) struct Dispatcher {
    shards: Vec<Shard>,
    next_shard: AtomicUsize,
}

/// The process-wide pool, started on first use.
///
/// Deliberately not built under `--cfg loom`: `loom::model` runs its closure
/// repeatedly in one process, so a pool started in one iteration would still
/// be running its workers, blocked on channels, in the next. Under Loom each
/// model builds and ends its own dispatcher instead.
#[cfg(not(loom))]
static POOL: Lazy<OnceCell<Dispatcher>> = Lazy::new(OnceCell::new);

/// Why the dispatch pool could not be started.
///
/// Returned by the logger constructors rather than panicking: a process at its
/// thread limit cannot have a pool, and that is a resource refusal the caller
/// can report, retry later, or run without — not a logic error.
#[derive(Debug)]
pub struct DispatcherError {
    /// How many shards were started before the refusal.
    started: usize,
    /// The operating system's refusal to start the next thread.
    source: std::io::Error,
}

impl std::fmt::Display for DispatcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "could not start the log dispatch pool: {} of {} shards started before the system \
             refused another thread: {}",
            self.started,
            shard_count(),
            self.source,
        )
    }
}

impl std::error::Error for DispatcherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(not(loom))]
impl Dispatcher {
    /// Return the process-wide pool, starting it if it is not running.
    ///
    /// # Errors
    ///
    /// Returns [`DispatcherError`] when the pool is not running and the system
    /// refuses to start its threads. The failure is not recorded, so a later
    /// caller retries — a process that has closed loggers may well be able to
    /// start the threads it could not start before.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let dispatcher = Dispatcher::shared()?;
    /// let shard = dispatcher.assign_shard();
    /// ```
    #[cfg(not(loom))]
    pub(crate) fn shared() -> Result<&'static Self, DispatcherError> {
        POOL.get_or_try_init(Self::start)
    }

    /// Assign a shard to a newly created logger.
    ///
    /// Round robin over a fixed pool spreads loggers evenly without a lock and
    /// without consulting the machine again.
    pub(crate) fn assign_shard(&self) -> &Shard {
        let index = self.next_shard.fetch_add(1, Ordering::Relaxed);
        &self.shards[index % self.shards.len()]
    }

    /// Start one shard per thread the machine can usefully run.
    fn start() -> Result<Self, DispatcherError> {
        let count = shard_count();
        let mut shards = Vec::with_capacity(count);
        for index in 0..count {
            let shard = Shard::start(index, RECORDS_PER_LOGGER * count).map_err(|source| {
                DispatcherError {
                    started: index,
                    source,
                }
            })?;
            shards.push(shard);
        }
        Ok(Self {
            shards,
            next_shard: AtomicUsize::new(0),
        })
    }
}

/// Number of shards to start, given the machine's available parallelism.
///
/// Pure and separate from the spawn so the clamp can be asserted without
/// starting threads, which is what keeps the pool's threads a function of the
/// machine rather than of how many loggers exist.
pub(crate) fn shards_for(available_parallelism: Option<usize>) -> usize {
    available_parallelism
        .unwrap_or(DEFAULT_SHARD_COUNT)
        .clamp(MIN_SHARD_COUNT, DEFAULT_SHARD_COUNT)
}

/// Number of shards this process should run.
pub(crate) fn shard_count() -> usize {
    shards_for(std::thread::available_parallelism().ok().map(|n| n.get()))
}

/// Start the process-wide pool from a test-controlled shard count.
#[cfg(all(test, not(loom)))]
pub(crate) use test_support::shared_with_shards_for_test;

/// The idle tick a shard's worker waits for, for tests that must outlast it.
///
/// Outside Loom only: the one caller is the drain-on-drop test, and a Loom
/// model has no clock to outlast.
#[cfg(all(test, not(loom)))]
pub(crate) fn idle_tick_for_test() -> std::time::Duration {
    shard::idle_tick_for_test()
}

#[cfg(all(test, not(loom)))]
mod test_support {
    //! Test-only entry points into the process-wide pool.
    //!
    //! Built only outside Loom: the whole point is to construct a [`Dispatcher`]
    //! at a chosen shard count, and a Loom model has no pool to construct.

    use super::{Dispatcher, Shard};

    /// A pool a test has started itself, with a shard count it chose.
    ///
    /// Returned rather than installed as the process-wide pool: the tests that
    /// want a specific shard count are asserting the pool's own behaviour, and
    /// installing it would make every other test in the binary share it.
    pub(crate) fn shared_with_shards_for_test(count: usize) -> Result<Dispatcher, std::io::Error> {
        let mut shards = Vec::with_capacity(count);
        for index in 0..count {
            shards.push(Shard::start(index, super::RECORDS_PER_LOGGER * count)?);
        }
        Ok(Dispatcher {
            shards,
            next_shard: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}
