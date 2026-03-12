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
    path: PathBuf,
    schedule: TimedRotationSchedule,
    backup_count: usize,
    clock: C,
    next_rollover_at: DateTime<Utc>,
}

impl TimedFileRotationStrategy<SystemClock> {
    pub(crate) fn new(path: PathBuf, schedule: TimedRotationSchedule, backup_count: usize) -> Self {
        Self::new_with_clock(path, schedule, backup_count, SystemClock)
    }
}

impl<C> TimedFileRotationStrategy<C>
where
    C: RotationClock,
{
    pub(crate) fn new_with_clock(
        path: PathBuf,
        schedule: TimedRotationSchedule,
        backup_count: usize,
        mut clock: C,
    ) -> Self {
        let now = clock.now();
        let next_rollover_at = schedule.next_rollover(now);
        Self {
            path,
            schedule,
            backup_count,
            clock,
            next_rollover_at,
        }
    }

    fn rotate(
        &mut self,
        writer: &mut BufWriter<File>,
        rollover_at: DateTime<Utc>,
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
                self.prune_backups()?;
                Ok(())
            }
            Err(err) => {
                fs::rename(&rotated_path, &self.path)?;
                *writer = BufWriter::with_capacity(capacity, original_file);
                Err(err)
            }
        }
    }

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

    fn open_fresh_writer(path: &Path, capacity: usize) -> io::Result<BufWriter<File>> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(BufWriter::with_capacity(capacity, file))
    }

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
            if has_os_prefix(&name, &prefix) {
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

    fn remove_file_if_exists(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}

impl crate::handlers::file::RotationStrategy<BufWriter<File>> for TimedFileRotationStrategy {
    fn before_write(&mut self, writer: &mut BufWriter<File>, _formatted: &str) -> io::Result<bool> {
        let now = self.clock.now();
        if now < self.next_rollover_at {
            return Ok(false);
        }
        let rollover_at = self.next_rollover_at;
        self.rotate(writer, rollover_at)?;
        self.next_rollover_at = self.schedule.next_rollover(now);
        Ok(true)
    }
}

fn has_os_prefix(value: &OsString, prefix: &OsString) -> bool {
    let value = value.to_string_lossy();
    let prefix = prefix.to_string_lossy();
    value.starts_with(prefix.as_ref())
}

/// Bundles timed-rotation parameters passed to the handler constructor.
pub(crate) struct TimedRotationConfig {
    pub(crate) schedule: TimedRotationSchedule,
    pub(crate) backup_count: usize,
}

/// File handler variant configured for timed rotation.
pub struct FemtoTimedRotatingFileHandler {
    inner: FemtoFileHandler,
    schedule: TimedRotationSchedule,
    backup_count: usize,
}

impl FemtoTimedRotatingFileHandler {
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

    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "python-only getter")
    )]
    pub(crate) fn schedule(&self) -> &TimedRotationSchedule {
        &self.schedule
    }

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
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path_ref)?;
        let writer = BufWriter::new(file);
        let rotation_strategy = TimedFileRotationStrategy::new(
            path_ref.to_path_buf(),
            rotation.schedule.clone(),
            rotation.backup_count,
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, TimeZone};
    use rstest::rstest;
    use tempfile::tempdir;

    use crate::{
        formatter::DefaultFormatter,
        handlers::{file::OverflowPolicy, timed_rotating::schedule::TimedRotationWhen},
        log_record::FemtoLogRecord,
    };

    fn utc_datetime(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .expect("test datetime must be valid")
    }

    #[rstest]
    fn rotates_and_prunes_backups() {
        let dir = tempdir().expect("tempdir must create a temporary directory");
        let path = dir.path().join("timed.log");
        let schedule =
            TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None).unwrap();
        let config = HandlerConfig {
            capacity: 8,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        };
        let mut handler = FemtoTimedRotatingFileHandler::with_capacity_flush_policy(
            &path,
            DefaultFormatter,
            config,
            TimedRotationConfig {
                schedule,
                backup_count: 1,
            },
        )
        .expect("timed handler must build");
        handler
            .handle(FemtoLogRecord::new(
                "logger",
                crate::level::FemtoLevel::Info,
                "hello",
            ))
            .expect("initial write must succeed");
        handler.close();
    }

    #[rstest]
    fn midnight_schedule_is_preserved() {
        let schedule = TimedRotationSchedule::new(
            TimedRotationWhen::Midnight,
            1,
            true,
            Some(NaiveTime::from_hms_opt(0, 0, 0).expect("midnight must be valid")),
        )
        .expect("midnight schedule must validate");

        let next = schedule.next_rollover(utc_datetime(2026, 3, 11, 23, 59, 59));

        assert_eq!(next, utc_datetime(2026, 3, 12, 0, 0, 0));
    }
}
