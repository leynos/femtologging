//! The time-based rollover strategy behind the timed rotating file handler.
//!
//! [`TimedFileRotationStrategy`] is what
//! [`FemtoTimedRotatingFileHandler`](super::FemtoTimedRotatingFileHandler)
//! installs as its
//! [`RotationStrategy`](crate::handlers::file::RotationStrategy). The handler's
//! worker calls it before every write, and it answers one question: has the
//! active file reached the deadline the cadence set, and if so what does
//! rotating it entail?
//!
//! Three collaborators answer that question, each with one reason to change:
//!
//! - [`RolloverSchedule`] holds the injectable [`RotationClock`] and the single
//!   deadline. It is seeded from the clock when the handler is built, from an
//!   existing log file's modification time when one predates that reading, and
//!   from the observation instant after a completed rotation.
//! - [rotation steps](super::rotation_steps) performs the rename and writer
//!   swap, restoring the original writer and undoing the rename when a step
//!   fails.
//! - [`BackupRetention`] prunes timestamped siblings beyond the configured
//!   count once a rotation has completed.
//!
//! Every filesystem step goes through
//! [`RotationFileStore`](super::rotation_store::RotationFileStore), so a test
//! substitutes a scripted store instead of arranging a directory that fails a
//! rename at the right moment. Rotation *orchestration* stays here: this type
//! is the only place that knows the order the steps take and which handler
//! state each one updates.
//!
//! The cadence itself — which suffixes are valid, and when the next rollover
//! falls — is [`TimedRotationSchedule`] from
//! [`super::schedule`]; this module consumes it rather than restating it. The
//! worker that calls the strategy lives in
//! [`super::handler`], which owns the handler's writer and asks for a rotation
//! before each write.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
};

use chrono::{DateTime, Utc};

use super::{
    backup_retention::{BackupRetention, backup_directory},
    clock::{RotationClock, SystemClock},
    rollover_schedule::{RolloverDue, RolloverSchedule},
    rotation_steps::{self, RotationParts},
    rotation_store::{RotationFileStore, StdFileStore},
    schedule::TimedRotationSchedule,
};

/// Rotation strategy for time-based file rollover.
///
/// The clock and the file store are the two injected seams; production uses
/// [`SystemClock`] and [`StdFileStore`], while tests substitute a deterministic
/// clock and a store that can fail a named step.
pub(crate) struct TimedFileRotationStrategy<C = SystemClock, S = StdFileStore> {
    /// Active log path renamed when a scheduled rollover occurs.
    path: PathBuf,
    /// Clock and deadline that decide when the next rollover happens.
    schedule: RolloverSchedule<C>,
    /// Which timestamped siblings survive a completed rotation.
    retention: BackupRetention,
    /// Filesystem seam every rotation step goes through.
    store: S,
}

impl TimedFileRotationStrategy<SystemClock, StdFileStore> {
    /// Creates a system-clock strategy for the supplied schedule and retention.
    pub(crate) fn new(path: PathBuf, schedule: TimedRotationSchedule, backup_count: usize) -> Self {
        Self::new_with_clock(path, schedule, backup_count, SystemClock)
    }
}

impl<C> TimedFileRotationStrategy<C, StdFileStore>
where
    C: RotationClock,
{
    /// Creates a strategy that samples `clock` for its rollover deadlines.
    pub(crate) fn new_with_clock(
        path: PathBuf,
        schedule: TimedRotationSchedule,
        backup_count: usize,
        clock: C,
    ) -> Self {
        Self::with_store(path, schedule, backup_count, clock, StdFileStore)
    }
}

impl<C, S> TimedFileRotationStrategy<C, S>
where
    C: RotationClock,
    S: RotationFileStore,
{
    /// Creates a strategy over an explicit clock and file store.
    ///
    /// The store is the seam tests replace; production reaches it through the
    /// two constructors above.
    pub(super) fn with_store(
        path: PathBuf,
        schedule: TimedRotationSchedule,
        backup_count: usize,
        clock: C,
        store: S,
    ) -> Self {
        let retention = BackupRetention::new(path.clone(), backup_count);
        Self {
            path,
            schedule: RolloverSchedule::new(schedule, clock),
            retention,
            store,
        }
    }

    /// The instant the strategy's schedule was seeded from.
    ///
    /// The handler compares an existing file's modification time against it,
    /// so a timestamp from the future cannot reseed the deadline.
    pub(crate) fn created_at(&self) -> DateTime<Utc> {
        self.schedule.created_at()
    }

    /// Re-seed the rollover deadline from an externally determined instant,
    /// such as an existing log file's modification time.
    pub(crate) fn seed_rollover_from(&mut self, seed: DateTime<Utc>) {
        self.schedule.seed_from(seed);
    }

    /// Re-seed from a log file's modification time, ignoring future mtimes.
    ///
    /// Only a seed that predates the clock's "now" is applied. A future mtime
    /// indicates a clock mismatch (for example injected test times against the
    /// real filesystem), so it is ignored rather than pushing the first
    /// rollover into the future.
    pub(crate) fn seed_rollover_from_mtime(&mut self, mtime: Option<DateTime<Utc>>) {
        if let Some(mtime) = mtime
            && mtime <= self.created_at()
        {
            self.seed_rollover_from(mtime);
        }
    }

    /// The pending deadline, for tests that assert how the handler seeded it.
    #[cfg(test)]
    pub(crate) fn next_rollover_at(&self) -> DateTime<Utc> {
        self.schedule.next_rollover_at()
    }

    /// Rotates the file at a deadline and restores the original writer if any
    /// rename or fresh-open step fails.
    fn rotate(&mut self, writer: &mut BufWriter<File>, due: RolloverDue) -> io::Result<()> {
        writer.flush()?;
        let capacity = writer.capacity();
        let placeholder_dir = backup_directory(&self.path);
        let original_file =
            rotation_steps::swap_writer_with_temp(&self.store, placeholder_dir, writer, capacity)?;
        let mut rotation = RotationParts {
            writer,
            capacity,
            original_file: Some(original_file),
            rotated_path: self.rotated_path(due.rollover_at),
        };
        rotation_steps::stage_rotated_file(&self.store, &self.path, &mut rotation)?;
        self.install_fresh_writer(rotation, due.now)
    }

    /// Open the replacement writer and finish the rotation's bookkeeping.
    ///
    /// The deadline advances before pruning so a prune failure does not cause
    /// repeated rollovers on the next write, and the writer is restored before
    /// the filesystem rollback so the handler stays usable even if undoing the
    /// rename also fails.
    fn install_fresh_writer(
        &mut self,
        mut rotation: RotationParts<'_>,
        now: DateTime<Utc>,
    ) -> io::Result<()> {
        match rotation_steps::open_fresh_writer(&self.store, &self.path, rotation.capacity) {
            Ok(fresh_writer) => {
                *rotation.writer = fresh_writer;
                self.schedule.advance_to(now);
                self.retention
                    .prune(&self.store, |suffix| self.schedule.is_valid_suffix(suffix))
            }
            Err(err) => {
                rotation.restore_writer();
                rotation_steps::rollback_rotation(
                    &self.store,
                    &self.path,
                    &rotation.rotated_path,
                    err,
                )
            }
        }
    }

    /// Builds the timestamped filename for a completed rotation period.
    fn rotated_path(&self, rollover_at: DateTime<Utc>) -> PathBuf {
        let suffix = self.schedule.suffix_for(rollover_at);
        let mut rotated = self.path.clone();
        let mut name = self
            .path
            .file_name()
            .map(|file_name| file_name.to_os_string())
            .unwrap_or_else(|| self.path.as_os_str().to_os_string());
        name.push(format!(".{suffix}"));
        rotated.set_file_name(name);
        rotated
    }
}

impl<C, S> crate::handlers::file::RotationStrategy<BufWriter<File>>
    for TimedFileRotationStrategy<C, S>
where
    C: RotationClock,
    S: RotationFileStore,
{
    fn before_write(&mut self, writer: &mut BufWriter<File>, _formatted: &str) -> io::Result<bool> {
        let Some(due) = self.schedule.due() else {
            return Ok(false);
        };
        self.rotate(writer, due)?;
        Ok(true)
    }
}
