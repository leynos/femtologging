//! When the next timed rollover is due.
//!
//! [`RolloverSchedule`] owns the injectable clock and the single deadline that
//! decides whether the next write rotates. Keeping both here gives the
//! deadline one owner and one reason to change: the cadence rules in
//! [`TimedRotationSchedule`] say *when* rollovers fall, and this type says
//! *which* of them the current file is waiting for.
//!
//! The deadline is seeded three ways and none of them belongs to rotation
//! itself: the clock reading taken when the handler is built, an existing log
//! file's modification time, and the instant a completed rotation advanced to.

use chrono::{DateTime, Utc};

use super::{clock::RotationClock, schedule::TimedRotationSchedule};

/// The clock and deadline that decide when the next rollover happens.
pub(super) struct RolloverSchedule<C> {
    /// Source of time owned by the handler's worker.
    clock: C,
    /// Validated cadence and timezone rules used to calculate deadlines.
    cadence: TimedRotationSchedule,
    /// Instant from which the current file's schedule was seeded.
    created_at: DateTime<Utc>,
    /// Next deadline at which the worker must rotate before writing.
    next_rollover_at: DateTime<Utc>,
}

/// A deadline a write reached, with the instant that observed it.
///
/// Rotation needs both: the deadline names the rotated file, while the
/// observation instant seeds the next one.
pub(super) struct RolloverDue {
    /// Deadline the active file has reached.
    pub(super) rollover_at: DateTime<Utc>,
    /// Clock reading that showed the deadline had passed.
    pub(super) now: DateTime<Utc>,
}

impl<C> RolloverSchedule<C>
where
    C: RotationClock,
{
    /// Samples the supplied clock and seeds the first rollover deadline from it.
    pub(super) fn new(cadence: TimedRotationSchedule, mut clock: C) -> Self {
        let seed = clock.now();
        let next_rollover_at = cadence.next_rollover(seed);
        Self {
            clock,
            cadence,
            created_at: seed,
            next_rollover_at,
        }
    }

    /// The instant the current file's schedule was seeded from.
    ///
    /// Callers compare an existing file's modification time against it so a
    /// timestamp from the future is ignored rather than reseeding the
    /// deadline with a clock mismatch.
    pub(super) fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Re-seed the deadline from an externally determined instant, such as an
    /// existing log file's modification time.
    pub(super) fn seed_from(&mut self, seed: DateTime<Utc>) {
        self.next_rollover_at = self.cadence.next_rollover(seed);
    }

    /// Report the deadline a write has reached, if it has reached one.
    ///
    /// This is the only place the deadline is compared against the clock, so
    /// a handler cannot rotate on one reading and label the rotation with
    /// another.
    pub(super) fn due(&mut self) -> Option<RolloverDue> {
        let now = self.clock.now();
        (now >= self.next_rollover_at).then_some(RolloverDue {
            rollover_at: self.next_rollover_at,
            now,
        })
    }

    /// Advance the deadline past a completed rotation.
    pub(super) fn advance_to(&mut self, now: DateTime<Utc>) {
        self.next_rollover_at = self.cadence.next_rollover(now);
    }

    /// The timestamped suffix a rotation at `rollover_at` acquires.
    pub(super) fn suffix_for(&self, rollover_at: DateTime<Utc>) -> String {
        self.cadence.suffix_for(rollover_at)
    }

    /// Whether `suffix` names a rotation this cadence produces.
    pub(super) fn is_valid_suffix(&self, suffix: &str) -> bool {
        self.cadence.is_valid_suffix(suffix)
    }

    /// The pending deadline, for tests that assert how a handler seeded it.
    #[cfg(test)]
    pub(super) fn next_rollover_at(&self) -> DateTime<Utc> {
        self.next_rollover_at
    }
}
