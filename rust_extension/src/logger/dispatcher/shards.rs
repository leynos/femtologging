//! The shards a running dispatch pool owns, in the order it started them.
//!
//! A pool has to hold at least one shard to assign one, and round-robin
//! assignment has to answer with a shard on every call. Both properties are
//! carried by the type: the first shard is stored apart from the rest, so an
//! empty pool is not representable, and a slot lookup past the end falls back
//! to that first shard instead of panicking.

use super::{DispatcherError, Shard};

/// A pool's shards, of which there is always at least one.
///
/// The emptiness a plain vector would allow is not representable here, which is
/// what lets [`super::Dispatcher::assign_shard`] answer with a shard on every
/// call rather than returning an option no caller could handle.
pub(super) struct Shards {
    /// The first shard, kept out of `rest` so one is always present.
    first: Shard,
    /// Every other shard, in the order the pool started them.
    rest: Vec<Shard>,
}

impl Shards {
    /// Build a pool from the shards started for it, in start order.
    ///
    /// # Errors
    ///
    /// Returns [`DispatcherError`] when `started` is empty. A pool with no
    /// shard can neither assign one to a logger nor dispatch a record, so the
    /// request is refused as invalid rather than deferred to the first
    /// assignment.
    pub(super) fn from_started(started: Vec<Shard>) -> Result<Self, DispatcherError> {
        let mut remaining = started.into_iter();
        let first = remaining.next().ok_or_else(|| {
            DispatcherError::invalid_pool(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a dispatch pool must own at least one shard",
            ))
        })?;
        Ok(Self {
            first,
            rest: remaining.collect(),
        })
    }

    /// Return how many shards the pool holds, which is never zero.
    pub(super) const fn len(&self) -> usize {
        // The `first` shard is stored apart from `rest`, so the count is one
        // more than the tail rather than the length of a single vector.
        1 + self.rest.len()
    }

    /// Return the shard in round-robin slot `slot`.
    ///
    /// Total by construction: slot zero is the first shard, and any slot past
    /// the end falls back to it instead of panicking. Callers derive the slot
    /// from [`Self::len`], so the fallback is defensive rather than expected.
    pub(super) fn get(&self, slot: usize) -> &Shard {
        slot.checked_sub(1)
            .and_then(|rest_index| self.rest.get(rest_index))
            .unwrap_or(&self.first)
    }
}
