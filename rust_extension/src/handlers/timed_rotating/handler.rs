//! The timed-rotating file handler wrapper.
//!
//! Rotation itself — when to roll over, how to rename the active file, and how
//! to prune old backups — lives in `core`. This module owns the public handler
//! that attaches a rotation strategy to a file handler and exposes its
//! lifecycle.

use std::{
    any::Any,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter},
    path::Path,
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use delegate::delegate;

use super::{
    core::TimedRotationConfig, rotation_strategy::TimedFileRotationStrategy,
    schedule::TimedRotationSchedule,
};
use crate::{
    formatter::FemtoFormatter,
    handler::{FemtoHandlerTrait, FlushContext, HandlerError},
    handlers::file::{BuilderOptions, FemtoFileHandler, HandlerConfig},
    log_record::FemtoLogRecord,
};

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
        let existing_mtime = existing_mtime(path_ref);
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
        rotation_strategy.seed_rollover_from_mtime(existing_mtime);
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

/// Read the modification time of an existing log file, if it has one.
///
/// The mtime is captured before the file is opened, because opening may create
/// it; seeding the first rollover from an existing file is what matches the
/// Python stdlib `TimedRotatingFileHandler` contract.
fn existing_mtime(path: &Path) -> Option<DateTime<Utc>> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| {
            let elapsed = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
            let secs = i64::try_from(elapsed.as_secs()).ok()?;
            DateTime::from_timestamp(secs, elapsed.subsec_nanos())
        })
}

impl FemtoHandlerTrait for FemtoTimedRotatingFileHandler {
    delegate! {
        to self.inner {
            fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError>;
            fn flush(&self) -> bool;
        }
    }

    fn flush_until(&self, context: &FlushContext) -> bool {
        self.inner.flush_by(context)
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
