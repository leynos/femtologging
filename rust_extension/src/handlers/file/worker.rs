//! Background worker thread for [`FemtoFileHandler`].
//!
//! This module owns the asynchronous file writing loop. The worker receives
//! `FileCommand` values over a channel, writes log records, flushes the
//! underlying writer, and notifies the handler when flushes complete. Tests
//! can spawn a worker with [`spawn_worker`] and inspect `FlushTracker` to
//! verify flushing behaviour.

use std::{
    io::{self, Seek, Write},
    sync::{Arc, Barrier},
    thread::{self, JoinHandle},
};

use crossbeam_channel::{Receiver, Sender, bounded};
use log::{error, warn};

use super::config::HandlerConfig;
use crate::{formatter::FemtoFormatter, log_record::FemtoLogRecord};

/// Commands sent to the worker thread.
pub enum FileCommand {
    Record(Box<FemtoLogRecord>),
    Flush,
}

pub trait RotationStrategy<W>: Send
where
    W: Write + Seek,
{
    fn before_write(&mut self, writer: &mut W, formatted: &str) -> io::Result<bool>;
}

/// Explicit strategy representing the absence of rotation logic.
///
/// A dedicated type avoids relying on the unit type to convey intent and keeps
/// error handling straightforward. The strategy guarantees it never reports an
/// error so the worker avoids logging spurious rotation failures when
/// rotation is disabled.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRotation;

impl<W: Write + Seek> RotationStrategy<W> for NoRotation {
    fn before_write(&mut self, _writer: &mut W, _formatted: &str) -> io::Result<bool> {
        Ok(false)
    }
}

/// Configuration for the background worker thread.
pub struct WorkerConfig {
    pub capacity: usize,
    pub flush_interval: usize,
    pub start_barrier: Option<Arc<Barrier>>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            capacity: super::config::DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            start_barrier: None,
        }
    }
}

impl From<&HandlerConfig> for WorkerConfig {
    fn from(cfg: &HandlerConfig) -> Self {
        Self {
            capacity: cfg.capacity,
            flush_interval: cfg.flush_interval,
            start_barrier: None,
        }
    }
}

/// Tracks how many writes occurred and triggers periodic flushes.
pub(crate) struct FlushTracker {
    writes: usize,
    flush_interval: usize,
}

impl FlushTracker {
    pub(crate) fn new(flush_interval: usize) -> Self {
        Self {
            writes: 0,
            flush_interval,
        }
    }

    pub(crate) fn record_write<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.writes += 1;
        self.flush_if_due(writer).map_err(|e| {
            warn!(
                "FemtoFileHandler flush error after write {}/{}: {e}",
                self.writes, self.flush_interval
            );
            e
        })?;
        Ok(())
    }

    pub(crate) fn reset(&mut self) {
        self.writes = 0;
    }

    /// Determines whether a flush should occur.
    ///
    /// A flush is due when the interval is positive, at least one write has
    /// occurred, and the write count is a multiple of the interval.
    fn should_flush(&self) -> bool {
        self.flush_interval != 0
            && self.writes > 0
            && self.writes.is_multiple_of(self.flush_interval)
    }

    fn flush_if_due<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.should_flush() {
            writer.flush()?;
        }
        Ok(())
    }
}

struct WorkerState<W, R> {
    writer: W,
    rotation: R,
    tracker: FlushTracker,
}

impl<W, R> WorkerState<W, R>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    fn new(writer: W, rotation: R, flush_interval: usize) -> Self {
        Self {
            writer,
            rotation,
            tracker: FlushTracker::new(flush_interval),
        }
    }

    fn handle_record<F>(&mut self, formatter: &F, record: FemtoLogRecord)
    where
        F: FemtoFormatter,
    {
        let message = formatter.format(&record);
        if let Err(err) = self.rotation.before_write(&mut self.writer, &message) {
            error!("FemtoFileHandler rotation error; writing record without rotating: {err}");
        }
        if let Err(err) =
            super::mod_impl::write_record(&mut self.writer, &message, &mut self.tracker)
        {
            warn!("FemtoFileHandler write error: {err}");
        }
    }

    fn handle_flush(&mut self, ack_tx: &Sender<()>) {
        if self.writer.flush().is_err() {
            warn!("FemtoFileHandler flush error");
        }
        self.tracker.reset();
        if ack_tx.send(()).is_err() {
            warn!("FemtoFileHandler flush ack channel disconnected");
        }
    }

    fn final_flush(&mut self) {
        if self.writer.flush().is_err() {
            warn!("FemtoFileHandler flush error");
        }
    }
}

pub fn spawn_worker<W, F, R>(
    writer: W,
    formatter: F,
    config: WorkerConfig,
    rotation: R,
) -> (
    Sender<FileCommand>,
    Receiver<()>,
    Receiver<()>,
    JoinHandle<()>,
)
where
    W: Write + Seek + Send + 'static,
    F: FemtoFormatter + Send + 'static,
    R: RotationStrategy<W> + Send + 'static,
{
    let WorkerConfig {
        capacity,
        flush_interval,
        start_barrier,
    } = config;
    let (tx, rx) = bounded(capacity);
    let (done_tx, done_rx) = bounded(1);
    let (ack_tx, ack_rx) = bounded(1);
    let handle = thread::spawn(move || {
        if let Some(b) = start_barrier {
            b.wait();
        }
        let mut state = WorkerState::new(writer, rotation, flush_interval);
        let formatter = formatter;
        for cmd in rx {
            match cmd {
                FileCommand::Record(record) => state.handle_record(&formatter, *record),
                FileCommand::Flush => state.handle_flush(&ack_tx),
            }
        }
        state.final_flush();
        let _ = done_tx.send(());
    });
    (tx, done_rx, ack_rx, handle)
}

#[cfg(test)]
mod flush_tracker_tests {
    use super::*;
    use crate::handlers::file::test_support;
    use rstest::*;
    use serial_test::serial;
    use std::io::{self, Write};

    #[derive(Default)]
    struct DummyWriter {
        flushed: usize,
        fail: bool,
    }

    impl Write for DummyWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed += 1;
            if self.fail {
                Err(io::Error::new(io::ErrorKind::Other, "flush failed"))
            } else {
                Ok(())
            }
        }
    }

    #[fixture]
    fn writer(#[default(false)] fail: bool) -> DummyWriter {
        DummyWriter { flushed: 0, fail }
    }

    #[rstest]
    #[case(2, 2, false, 1, false)]
    #[case(1, 1, true, 1, true)]
    #[case(3, 1, false, 0, false)]
    #[case(0, 5, false, 0, false)]
    #[case(2, 0, false, 0, false)]
    fn flush_if_due_cases(
        #[case] interval: usize,
        #[case] writes: usize,
        #[case] _fail: bool,
        #[case] expected_flushes: usize,
        #[case] expect_error: bool,
        #[with(_fail)] mut writer: DummyWriter,
    ) {
        let mut tracker = FlushTracker::new(interval);
        tracker.writes = writes;
        let result = tracker.flush_if_due(&mut writer);
        assert_eq!(writer.flushed, expected_flushes);
        assert_eq!(result.is_err(), expect_error);
    }

    #[rstest]
    #[serial]
    fn record_write_logs_warning_on_error(#[with(true)] mut writer: DummyWriter) {
        test_support::install_test_logger();
        let mut tracker = FlushTracker::new(1);
        let result = tracker.record_write(&mut writer);
        assert!(result.is_err());
        assert_eq!(writer.flushed, 1);

        let logs = test_support::take_logged_messages();
        let log = logs.into_iter().next().expect("no log produced");
        assert_eq!(log.level, log::Level::Warn);
        assert!(log.message.contains("after write"));
        assert!(log.message.contains("flush failed"));
    }
}
