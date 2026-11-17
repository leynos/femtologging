//! The timed rotating handler type and its constructor.
//!
//! The rotation strategy in [`super::core`] owns the rollover mechanics; this
//! module owns the handler that drives it, so each file stays inside the
//! module-length budget.

use std::{
    any::Any,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter},
    path::Path,
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use delegate::delegate;

use super::{core::TimedFileRotationStrategy, schedule::TimedRotationSchedule};
use crate::{
    formatter::FemtoFormatter,
    handler::{FemtoHandlerTrait, HandlerError},
    handlers::file::{BuilderOptions, FemtoFileHandler, HandlerConfig},
    log_record::FemtoLogRecord,
};

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
    pub(super) inner: FemtoFileHandler,
    /// Schedule retained for Python getters and worker construction.
    schedule: TimedRotationSchedule,
    /// Retention count exposed by the Python-facing timed handler.
    backup_count: usize,
}

impl FemtoTimedRotatingFileHandler {
    /// Wraps an existing file handler with timed-rotation metadata.
    pub(crate) const fn new_with_schedule(
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
    pub(crate) const fn schedule(&self) -> &TimedRotationSchedule {
        &self.schedule
    }

    /// Returns the configured number of timestamped backups.
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub(crate) const fn backup_count(&self) -> usize {
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
        // Capture mtime before opening (which may create) the file so the first
        // rollover can seed from it, matching the Python stdlib contract.
        let existing_mtime = file_mtime(path_ref);
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
        // Apply the mtime seed only when it predates the clock's "now"; a future
        // mtime indicates a clock mismatch and must be ignored.
        if let Some(mtime) = existing_mtime
            && mtime <= rotation_strategy.seeded_at()
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
            /// Close the handler; concurrent callers share one completion wait.
            pub fn close(&self);
        }
    }
}

/// Return a file's modification time, or `None` when it cannot be read.
///
/// A file that is missing, unreadable, or older than the epoch is treated as
/// undated: the handler then seeds its schedule from the clock rather than
/// refusing to open.
fn file_mtime(path: &Path) -> Option<DateTime<Utc>> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let elapsed = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let secs = i64::try_from(elapsed.as_secs()).ok()?;
    DateTime::from_timestamp(secs, elapsed.subsec_nanos())
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
