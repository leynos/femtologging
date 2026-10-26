//! Core timed rotating file handler logic.
//!
//! This module keeps worker-thread rollover logic and filesystem retention
//! separate from the Python bindings.

use std::{
    any::Any,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter},
    path::Path,
    time::{Instant, SystemTime},
};

use chrono::DateTime;
use delegate::delegate;

use super::{rotation_strategy::TimedFileRotationStrategy, schedule::TimedRotationSchedule};
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
        }
    }
    fn flush(&self) -> bool {
        self.inner.flush()
    }

    fn flush_until(&self, deadline: Instant) -> bool {
        self.inner.flush_by(deadline)
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
