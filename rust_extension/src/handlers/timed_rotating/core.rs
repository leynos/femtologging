//! Core timed rotating file handler logic.
//!
//! This module keeps worker-thread rollover logic and filesystem retention
//! separate from the Python bindings. The public
//! `FemtoTimedRotatingFileHandler` wrapper that owns the worker lives in
//! `handler.rs`; this module supplies the rotation mechanics and
//! `TimedRotationConfig` it uses.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use tempfile::NamedTempFile;

use super::{
    clock::{RotationClock, SystemClock},
    schedule::TimedRotationSchedule,
};

/// Rotation strategy for time-based file rollover.
pub(crate) struct TimedFileRotationStrategy<C = SystemClock> {
    /// Active log path renamed when a scheduled rollover occurs.
    path: PathBuf,
    /// Validated cadence and timezone rules used to calculate deadlines.
    schedule: TimedRotationSchedule,
    /// Maximum number of timestamped files retained after rotation.
    backup_count: usize,
    /// Clock source owned by the worker, with a deterministic test implementation.
    clock: C,
    /// Instant from which the current file's schedule was seeded.
    created_at: DateTime<Utc>,
    /// Next deadline at which the worker must rotate before writing.
    next_rollover_at: DateTime<Utc>,
}

impl TimedFileRotationStrategy<SystemClock> {
    /// Creates a system-clock strategy for the supplied schedule and retention.
    pub(crate) fn new(path: PathBuf, schedule: TimedRotationSchedule, backup_count: usize) -> Self {
        Self::new_with_clock(path, schedule, backup_count, SystemClock)
    }
}

impl<C> TimedFileRotationStrategy<C>
where
    C: RotationClock,
{
    /// Samples the supplied clock and seeds the first rollover deadline from that instant.
    pub(crate) fn new_with_clock(
        path: PathBuf,
        schedule: TimedRotationSchedule,
        backup_count: usize,
        mut clock: C,
    ) -> Self {
        let seed = clock.now();
        let next_rollover_at = schedule.next_rollover(seed);
        Self {
            path,
            schedule,
            backup_count,
            clock,
            created_at: seed,
            next_rollover_at,
        }
    }

    /// Re-seed the deadline from an externally determined instant, such as a file mtime.
    pub(crate) fn seed_rollover_from(&mut self, seed: DateTime<Utc>) {
        self.next_rollover_at = self.schedule.next_rollover(seed);
    }

    /// Re-seed from a log file's modification time, ignoring future mtimes.
    ///
    /// Only a seed that predates the clock's "now" is applied. A future mtime
    /// indicates a clock mismatch (for example injected test times against the
    /// real filesystem), so it is ignored rather than pushing the first
    /// rollover into the future.
    pub(crate) fn seed_rollover_from_mtime(&mut self, mtime: Option<DateTime<Utc>>) {
        if let Some(mtime) = mtime
            && mtime <= self.created_at
        {
            self.seed_rollover_from(mtime);
        }
    }

    #[cfg(test)]
    pub(crate) const fn next_rollover_at(&self) -> DateTime<Utc> {
        self.next_rollover_at
    }

    /// Rotates the file at a deadline and restores the original writer if any rename or fresh-open step fails.
    fn rotate(
        &mut self,
        writer: &mut BufWriter<File>,
        rollover_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> io::Result<()> {
        writer.flush()?;
        let capacity = writer.capacity();
        let original_file = self.swap_writer_with_temp(writer, capacity)?;
        let rotated_path = self.rotated_path(rollover_at);
        let original_file =
            self.rename_active_file(writer, capacity, original_file, &rotated_path)?;

        match Self::open_fresh_writer(&self.path, capacity) {
            Ok(fresh_writer) => {
                drop(original_file);
                *writer = fresh_writer;
                self.advance_rollover(now)
            }
            Err(err) => {
                self.restore_after_failed_open(writer, capacity, original_file, &rotated_path, err)
            }
        }
    }

    /// Move the active file aside, restoring `writer` if either step fails.
    ///
    /// The original file is handed back on success because the caller still
    /// needs it if opening the fresh writer fails.
    fn rename_active_file(
        &self,
        writer: &mut BufWriter<File>,
        capacity: usize,
        original_file: File,
        rotated_path: &Path,
    ) -> io::Result<File> {
        if let Err(err) = Self::remove_file_if_exists(rotated_path) {
            *writer = BufWriter::with_capacity(capacity, original_file);
            return Err(err);
        }

        if let Err(err) = fs::rename(&self.path, rotated_path) {
            *writer = BufWriter::with_capacity(capacity, original_file);
            return Err(err);
        }

        Ok(original_file)
    }

    /// Advance the rollover deadline and prune old backups.
    ///
    /// The deadline moves before pruning so a prune failure does not cause
    /// repeated rollovers on the next write.
    fn advance_rollover(&mut self, now: DateTime<Utc>) -> io::Result<()> {
        self.next_rollover_at = self.schedule.next_rollover(now);
        self.prune_backups()
    }

    /// Restore the original writer after a failed fresh-writer open.
    ///
    /// The writer is restored before the filesystem rollback so the handler
    /// remains usable even if the rename also fails.
    fn restore_after_failed_open(
        &mut self,
        writer: &mut BufWriter<File>,
        capacity: usize,
        original_file: File,
        rotated_path: &Path,
        open_err: io::Error,
    ) -> io::Result<()> {
        *writer = BufWriter::with_capacity(capacity, original_file);
        if let Err(rollback_err) = fs::rename(rotated_path, &self.path) {
            return Err(io::Error::new(
                open_err.kind(),
                format!(
                    "failed to open fresh writer: {open_err}; rollback rename also failed: {rollback_err}"
                ),
            ));
        }
        Err(open_err)
    }

    /// Temporarily swaps in a placeholder writer so the active file can be
    /// renamed without losing the original on extraction failure.
    fn swap_writer_with_temp(
        &self,
        writer: &mut BufWriter<File>,
        capacity: usize,
    ) -> io::Result<File> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        let temp = NamedTempFile::new_in(dir)?;
        let placeholder_file = temp.reopen()?;
        drop(temp);
        let placeholder = BufWriter::with_capacity(capacity, placeholder_file);
        let original_writer = std::mem::replace(writer, placeholder);
        match original_writer.into_inner() {
            Ok(file) => Ok(file),
            Err(err) => {
                let io_error = io::Error::new(err.error().kind(), err.error().to_string());
                let original = err.into_inner();
                *writer = original;
                Err(io_error)
            }
        }
    }

    /// Opens a truncated writer for the active path after a rollover.
    fn open_fresh_writer(path: &Path, capacity: usize) -> io::Result<BufWriter<File>> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(BufWriter::with_capacity(capacity, file))
    }

    /// Builds the timestamped filename for a completed rotation period.
    fn rotated_path(&self, rollover_at: DateTime<Utc>) -> PathBuf {
        let suffix = self.schedule.suffix_for(rollover_at);
        let mut rotated = self.path.clone();
        let mut name = self.path.file_name().map_or_else(
            || self.path.as_os_str().to_os_string(),
            std::ffi::OsStr::to_os_string,
        );
        name.push(format!(".{suffix}"));
        rotated.set_file_name(name);
        rotated
    }

    /// Check whether a filename is a rotated backup for this schedule.
    fn matches_rotated_file_name(&self, name: &OsString, prefix: &OsString) -> bool {
        if !has_os_prefix(name, prefix) {
            return false;
        }
        let file_name_lossy = name.to_string_lossy();
        let prefix_lossy = prefix.to_string_lossy();
        let Some(suffix) = file_name_lossy.strip_prefix(prefix_lossy.as_ref()) else {
            return false;
        };
        self.schedule.is_valid_suffix(suffix)
    }

    /// Removes timestamped files beyond the configured retention count.
    fn prune_backups(&self) -> io::Result<()> {
        if self.backup_count == 0 {
            return Ok(());
        }
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let base_name = self.path.file_name().map_or_else(
            || self.path.as_os_str().to_os_string(),
            std::ffi::OsStr::to_os_string,
        );
        let mut prefix = base_name.clone();
        prefix.push(".");
        let mut backups = Vec::new();
        for entry in fs::read_dir(parent)? {
            let dir_entry = entry?;
            let name = dir_entry.file_name();
            if self.matches_rotated_file_name(&name, &prefix) {
                backups.push(dir_entry.path());
            }
        }
        backups.sort();
        while backups.len() > self.backup_count {
            let oldest = backups.remove(0);
            Self::remove_file_if_exists(&oldest)?;
        }
        Ok(())
    }

    /// Treats an absent file as already removed, propagating other failures.
    fn remove_file_if_exists(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}

impl<C> crate::handlers::file::RotationStrategy<BufWriter<File>> for TimedFileRotationStrategy<C>
where
    C: RotationClock,
{
    fn before_write(&mut self, writer: &mut BufWriter<File>, _formatted: &str) -> io::Result<bool> {
        let now = self.clock.now();
        if now < self.next_rollover_at {
            return Ok(false);
        }
        let rollover_at = self.next_rollover_at;
        self.rotate(writer, rollover_at, now)?;
        Ok(true)
    }
}

/// Compares encoded path prefixes without requiring UTF-8 conversion.
pub(super) fn has_os_prefix(value: &OsString, prefix: &OsString) -> bool {
    value
        .as_os_str()
        .as_encoded_bytes()
        .starts_with(prefix.as_os_str().as_encoded_bytes())
}

/// Bundles timed-rotation parameters passed to the handler constructor.
pub(crate) struct TimedRotationConfig {
    /// Validated schedule used by the worker before each record write.
    pub(crate) schedule: TimedRotationSchedule,
    /// Number of timestamped backups retained by the timed handler.
    pub(crate) backup_count: usize,
}

#[cfg(test)]
mod tests {
    //! Tests for rollback after a failed fresh-writer open.

    use std::fs::File;
    use std::io::{self, BufWriter};

    use tempfile::tempdir;

    use super::TimedFileRotationStrategy;
    use crate::handlers::timed_rotating::schedule::{TimedRotationSchedule, TimedRotationWhen};

    #[test]
    fn restore_after_failed_open_reports_a_failed_rename() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("timed.log");
        let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
            .expect("seconds schedule must validate");
        let mut strategy = TimedFileRotationStrategy::new(path.clone(), schedule, 1);
        let mut writer = BufWriter::new(File::create(&path).expect("active file"));
        let original = File::create(dir.path().join("original")).expect("original file");
        let missing = dir.path().join("missing");
        let err = strategy
            .restore_after_failed_open(&mut writer, 8, original, &missing, io::Error::other("boom"))
            .expect_err("missing rotated file must fail the rename");
        assert!(err.to_string().contains("rollback rename also failed"));
    }
}
