//! Core timed rotating file handler logic.
//!
//! This module keeps worker-thread rollover logic and filesystem retention
//! separate from the Python bindings.

use std::{
    any::Any,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use delegate::delegate;
use tempfile::NamedTempFile;

use super::{
    clock::{RotationClock, SystemClock},
    schedule::TimedRotationSchedule,
};
use crate::{
    formatter::FemtoFormatter,
    handler::{FemtoHandlerTrait, HandlerError},
    handlers::file::{BuilderOptions, FemtoFileHandler, HandlerConfig},
    log_record::FemtoLogRecord,
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
    /// Builds strategy state from a known seed so tests and mtime recovery share the same deadline calculation.
    fn from_seed(
        path: PathBuf,
        schedule: TimedRotationSchedule,
        backup_count: usize,
        clock: C,
        seed: DateTime<Utc>,
    ) -> Self {
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

    /// Samples the supplied clock and seeds the first rollover deadline from that instant.
    pub(crate) fn new_with_clock(
        path: PathBuf,
        schedule: TimedRotationSchedule,
        backup_count: usize,
        mut clock: C,
    ) -> Self {
        let seed = clock.now();
        Self::from_seed(path, schedule, backup_count, clock, seed)
    }

    /// Re-seed `next_rollover_at` from an externally determined instant, such as
    /// an existing log file's modification time.
    pub(crate) fn seed_rollover_from(&mut self, seed: DateTime<Utc>) {
        self.next_rollover_at = self.schedule.next_rollover(seed);
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
        let rotated_path = self.rotated_path(rollover_at);

        if let Err(err) = Self::remove_file_if_exists(&rotated_path) {
            *writer = BufWriter::with_capacity(capacity, original_file);
            return Err(err);
        }

        if let Err(err) = fs::rename(&self.path, &rotated_path) {
            *writer = BufWriter::with_capacity(capacity, original_file);
            return Err(err);
        }

        match Self::open_fresh_writer(&self.path, capacity) {
            Ok(fresh_writer) => {
                let _ = original_file;
                *writer = fresh_writer;
                // Advance the rollover deadline before pruning so a prune failure
                // does not cause repeated rollovers on the next write.
                self.next_rollover_at = self.schedule.next_rollover(now);
                self.prune_backups()?;
                Ok(())
            }
            Err(err) => {
                // Restore the writer before filesystem rollback so the handler
                // remains usable even if the rename also fails.
                *writer = BufWriter::with_capacity(capacity, original_file);
                if let Err(rollback_err) = fs::rename(&rotated_path, &self.path) {
                    return Err(io::Error::new(
                        err.kind(),
                        format!(
                            "failed to open fresh writer: {err}; rollback rename also failed: {rollback_err}"
                        ),
                    ));
                }
                Err(err)
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
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
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

/// File handler variant configured for timed rotation.
pub struct FemtoTimedRotatingFileHandler {
    /// Core file handler used for record delivery and lifecycle operations.
    inner: FemtoFileHandler,
    /// Schedule retained for Python getters and worker construction.
    schedule: TimedRotationSchedule,
    /// Retention count exposed by the Python-facing timed handler.
    backup_count: usize,
}

impl FemtoTimedRotatingFileHandler {
    /// Wraps an existing file handler with timed-rotation metadata.
    pub(crate) fn new_with_schedule(
        inner: FemtoFileHandler,
        schedule: TimedRotationSchedule,
        backup_count: usize,
    ) -> Self {
        Self {
            inner,
            schedule,
            backup_count,
        }
    }

    /// Returns the validated schedule used by this handler.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub(crate) fn schedule(&self) -> &TimedRotationSchedule {
        &self.schedule
    }

    /// Returns the configured number of timestamped backups.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub(crate) fn backup_count(&self) -> usize {
        self.backup_count
    }
    /// Build a timed rotating handler with the supplied configuration.
    pub(crate) fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
        rotation: TimedRotationConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let path_ref = path.as_ref();
        // Capture mtime before opening (which may create) the file so we
        // can seed the first rollover from it, matching the Python stdlib
        // TimedRotatingFileHandler contract.
        let existing_mtime = fs::metadata(path_ref)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|st| {
                let d = st.duration_since(SystemTime::UNIX_EPOCH).ok()?;
                let secs = i64::try_from(d.as_secs()).ok()?;
                DateTime::from_timestamp(secs, d.subsec_nanos())
            });
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path_ref)?;
        let writer = BufWriter::new(file);
        let mut rotation_strategy = TimedFileRotationStrategy::new(
            path_ref.to_path_buf(),
            rotation.schedule.clone(),
            rotation.backup_count,
        );
        // Only apply the mtime seed when it predates the clock's "now"
        // (which new() already consumed). A future mtime indicates a
        // clock mismatch (e.g. injected test times vs. real filesystem)
        // and must be ignored.
        if let Some(mtime) = existing_mtime
            && mtime <= rotation_strategy.created_at
        {
            rotation_strategy.seed_rollover_from(mtime);
        }
        let options = BuilderOptions::<BufWriter<File>, _>::new(rotation_strategy, None);
        let handler = FemtoFileHandler::build_from_worker(writer, formatter, config, options);
        Ok(Self::new_with_schedule(
            handler,
            rotation.schedule,
            rotation.backup_count,
        ))
    }
    delegate! {
        to self.inner {
            /// Flush any queued log records.
            pub fn flush(&self) -> bool;
            /// Close the handler, waiting for the worker thread to shut down.
            pub fn close(&mut self);
        }
    }
}
impl FemtoHandlerTrait for FemtoTimedRotatingFileHandler {
    delegate! {
        to self.inner {
            fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError>;
            fn flush(&self) -> bool;
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for FemtoTimedRotatingFileHandler {
    fn drop(&mut self) {
        self.close();
    }
}
