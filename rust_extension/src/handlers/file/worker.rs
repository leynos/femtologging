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

use crossbeam_channel::{bounded, Receiver, Sender};
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
    fn before_write(&mut self, writer: &mut W, formatted: &str) -> io::Result<()>;
}

impl<W: Write + Seek> RotationStrategy<W> for () {
    fn before_write(&mut self, _writer: &mut W, _formatted: &str) -> io::Result<()> {
        Ok(())
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

pub fn spawn_worker<W, F, R>(
    writer: W,
    formatter: F,
    config: WorkerConfig,
    ack_tx: Sender<()>,
    rotation: Option<R>,
) -> (Sender<FileCommand>, Receiver<()>, JoinHandle<()>)
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
    let handle = thread::spawn(move || {
        let mut rotation = rotation;
        if let Some(b) = start_barrier {
            b.wait();
        }
        let mut writer = writer;
        let formatter = formatter;
        let mut tracker = FlushTracker::new(flush_interval);
        for cmd in rx {
            match cmd {
                FileCommand::Record(record) => {
                    let record = *record;
                    let message = formatter.format(&record);
                    let rotation_result = rotation
                        .as_mut()
                        .map(|strategy| strategy.before_write(&mut writer, &message))
                        .unwrap_or(Ok(()));
                    if let Err(err) = rotation_result {
                        error!("FemtoFileHandler rotation error: {err}");
                        continue;
                    }
                    if let Err(e) =
                        super::mod_impl::write_record(&mut writer, &message, &mut tracker)
                    {
                        warn!("FemtoFileHandler write error: {e}");
                    }
                }
                FileCommand::Flush => {
                    if writer.flush().is_err() {
                        warn!("FemtoFileHandler flush error");
                    }
                    tracker.reset();
                    let _ = ack_tx.send(());
                }
            }
        }
        if writer.flush().is_err() {
            warn!("FemtoFileHandler flush error");
        }
        let _ = done_tx.send(());
    });
    (tx, done_rx, handle)
}

#[cfg(test)]
mod flush_tracker_tests {
    use super::*;
    use logtest::Logger;
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
        let mut logger = Logger::start();
        let mut tracker = FlushTracker::new(1);
        let result = tracker.record_write(&mut writer);
        assert!(result.is_err());
        assert_eq!(writer.flushed, 1);

        let log = logger.pop().expect("no log produced");
        assert_eq!(log.level(), log::Level::Warn);
        assert!(log.args().contains("after write"));
        assert!(log.args().contains("flush failed"));
    }
}
