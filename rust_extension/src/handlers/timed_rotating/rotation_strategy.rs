//! Timed rotating file strategy for scheduling and rotating log files.

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
    pub(super) created_at: DateTime<Utc>,
    /// Next deadline at which the worker must rotate before writing.
    next_rollover_at: DateTime<Utc>,
}

impl TimedFileRotationStrategy<SystemClock> {
    /// Creates a system-clock strategy for the supplied schedule and retention.
    pub(crate) fn new(path: PathBuf, schedule: TimedRotationSchedule, backup_count: usize) -> Self {
        Self::new_with_clock(path, schedule, backup_count, SystemClock)
    }
}

/// The state one rotation carries between its steps.
///
/// A rotation replaces the handler's writer with a temporary one so the active
/// file can be renamed. Every later step needs the same four pieces to finish
/// or to put the handler back as it was, so they travel together rather than as
/// a parameter list each step has to unpack.
struct RotationParts<'a> {
    /// The handler's writer, rebuilt from the original file if a step fails.
    writer: &'a mut BufWriter<File>,
    /// Buffer capacity used when the writer is rebuilt.
    capacity: usize,
    /// The file the writer held before the temporary swap, until it is restored
    /// or deliberately dropped.
    original_file: Option<File>,
    /// Timestamped destination the active file is renamed onto.
    rotated_path: PathBuf,
}

impl RotationParts<'_> {
    /// Put the original file back, so the handler keeps working after a failed
    /// rotation step.
    fn restore_writer(&mut self) {
        if let Some(file) = self.original_file.take() {
            *self.writer = BufWriter::with_capacity(self.capacity, file);
        }
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

    /// Re-seed `next_rollover_at` from an externally determined instant, such as
    /// an existing log file's modification time.
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
    pub(crate) fn next_rollover_at(&self) -> DateTime<Utc> {
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
        let mut rotation = RotationParts {
            writer,
            capacity,
            original_file: Some(original_file),
            rotated_path: self.rotated_path(rollover_at),
        };
        self.stage_rotated_file(&mut rotation)?;
        self.install_fresh_writer(rotation, now)
    }

    /// Clear the previous rotation target and rename the active file onto it.
    ///
    /// Any failure restores the original file as the active writer, so the
    /// caller observes a handler that is still usable and unchanged on disk.
    fn stage_rotated_file(&self, rotation: &mut RotationParts<'_>) -> io::Result<()> {
        if let Err(err) = Self::remove_file_if_exists(&rotation.rotated_path) {
            rotation.restore_writer();
            return Err(err);
        }
        if let Err(err) = fs::rename(&self.path, &rotation.rotated_path) {
            rotation.restore_writer();
            return Err(err);
        }
        Ok(())
    }

    /// Open the replacement writer, undoing the rename if that fails.
    fn install_fresh_writer(
        &mut self,
        mut rotation: RotationParts<'_>,
        now: DateTime<Utc>,
    ) -> io::Result<()> {
        match Self::open_fresh_writer(&self.path, rotation.capacity) {
            Ok(fresh_writer) => {
                *rotation.writer = fresh_writer;
                // Advance the rollover deadline before pruning so a prune failure
                // does not cause repeated rollovers on the next write.
                self.next_rollover_at = self.schedule.next_rollover(now);
                self.prune_backups()
            }
            Err(err) => {
                // Restore the writer before filesystem rollback so the handler
                // remains usable even if the rename also fails.
                rotation.restore_writer();
                rollback_rotation(&self.path, &rotation.rotated_path, err)
            }
        }
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
        let mut name = self
            .path
            .file_name()
            .map(|file_name| file_name.to_os_string())
            .unwrap_or_else(|| self.path.as_os_str().to_os_string());
        name.push(format!(".{suffix}"));
        rotated.set_file_name(name);
        rotated
    }

    /// Check if a filename matches the expected pattern for a rotated log file.
    ///
    /// Returns `true` only if the filename starts with the expected prefix
    /// and has a valid suffix for this schedule's rotation cadence.
    fn matches_rotated_file_name(&self, name: &OsString, prefix: &OsString) -> bool {
        if !has_os_prefix(name, prefix) {
            return false;
        }
        let name = name.to_string_lossy();
        let prefix = prefix.to_string_lossy();
        let Some(suffix) = name.strip_prefix(prefix.as_ref()) else {
            return false;
        };
        self.schedule.is_valid_suffix(suffix)
    }

    /// Removes timestamped files beyond the configured retention count.
    fn prune_backups(&self) -> io::Result<()> {
        if self.backup_count == 0 {
            return Ok(());
        }
        let parent = backup_directory(&self.path);
        let base_name = self
            .path
            .file_name()
            .map(|value| value.to_os_string())
            .unwrap_or_else(|| self.path.as_os_str().to_os_string());
        let prefix = {
            let mut value = base_name.clone();
            value.push(".");
            value
        };
        let mut backups = Vec::new();
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            if self.matches_rotated_file_name(&name, &prefix) {
                backups.push(entry.path());
            }
        }
        backups.sort();
        while backups.len() > self.backup_count {
            let oldest = backups.remove(0);
            Self::remove_file_if_exists(&oldest)?;
        }
        Ok(())
    }

    /// Treats an absent file as already removed while propagating other I/O
    /// failures.
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

/// Undo a rotation rename, reporting both failures when the undo also fails.
///
/// The open error stays the cause so callers see why rotation started, while
/// the message names the rollback failure that left the handler degraded.
fn rollback_rotation(active: &Path, rotated: &Path, cause: io::Error) -> io::Result<()> {
    if let Err(rollback_err) = fs::rename(rotated, active) {
        return Err(io::Error::new(
            cause.kind(),
            format!(
                "failed to open fresh writer: {cause}; rollback rename also failed: {rollback_err}"
            ),
        ));
    }
    Err(cause)
}

/// Compares encoded path prefixes without requiring UTF-8 conversion.
pub(super) fn has_os_prefix(value: &OsString, prefix: &OsString) -> bool {
    value
        .as_os_str()
        .as_encoded_bytes()
        .starts_with(prefix.as_os_str().as_encoded_bytes())
}

/// The directory that holds a log path's rotated siblings.
///
/// A single-component path such as `app.log` has a parent of `""`, not `None`,
/// and `read_dir("")` fails on Unix. Retention must therefore treat an empty
/// parent as the current directory — which is what the empty string means —
/// or it silently skips every rollover.
fn backup_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    //! Unit tests for locating the directory a rotation's backups live in.

    use super::backup_directory;
    use std::path::Path;

    /// A bare filename must scan the current directory, not the empty path.
    #[test]
    fn a_bare_file_name_scans_the_current_directory() {
        assert_eq!(backup_directory(Path::new("app.log")), Path::new("."));
    }

    /// A directory component is kept as it is written.
    #[test]
    fn a_parent_directory_is_preserved() {
        assert_eq!(
            backup_directory(Path::new("logs/app.log")),
            Path::new("logs")
        );
        assert_eq!(
            backup_directory(Path::new("/var/log/app.log")),
            Path::new("/var/log")
        );
    }
}
