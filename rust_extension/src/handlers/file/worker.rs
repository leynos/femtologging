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

use crossbeam_channel::{Receiver, RecvError, Sender, bounded};
use log::{error, warn};

use super::config::HandlerConfig;
use crate::{formatter::FemtoFormatter, log_record::FemtoLogRecord};

/// Bounds the non-blocking drain after the worker receives its first command.
pub(crate) const DEFAULT_BATCH_CAPACITY: usize = 64;

/// Commands sent to the worker thread.
///
/// The worker processes these commands in sequence, writing records or
/// performing explicit flushes as directed.
pub enum FileCommand {
    /// Write a log record to the underlying writer.
    Record(Box<FemtoLogRecord>),
    /// Flush the writer and acknowledge completion on the provided channel.
    Flush(Sender<io::Result<()>>),
}

/// Strategy for rotating the log file before writes.
///
/// Implementations can inspect the formatted message and writer state to decide
/// whether rotation is needed. The worker calls `before_write` before each record
/// is written.
pub trait RotationStrategy<W>: Send
where
    W: Write + Seek,
{
    /// Inspect the writer and message; rotate if needed.
    ///
    /// Returns `Ok(true)` if rotation occurred, `Ok(false)` if no rotation was needed,
    /// or `Err` if rotation failed.
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

/// Configuration for batch draining in the worker loop.
///
/// `capacity` is the maximum number of commands processed in one blocking
/// receive plus non-blocking drain cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchConfig {
    /// Limits the number of commands handled in one receive-and-drain cycle.
    capacity: usize,
}

/// Reports why a worker batch could not be assembled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchConfigError {
    /// The requested batch capacity was zero.
    ZeroCapacity,
}

impl BatchConfig {
    /// Create a batch configuration with a non-zero drain capacity.
    pub fn new(capacity: usize) -> Result<Self, BatchConfigError> {
        if capacity == 0 {
            return Err(BatchConfigError::ZeroCapacity);
        }
        Ok(Self { capacity })
    }

    /// Return the maximum number of commands drained in one batch.
    pub const fn capacity(self) -> usize {
        self.capacity
    }
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_BATCH_CAPACITY,
        }
    }
}

/// Configuration for the background worker thread.
///
/// Specifies the channel `capacity`, batch-drain configuration, `flush_interval`,
/// and optional synchronisation `start_barrier` for tests.
pub struct WorkerConfig {
    /// Capacity of the command channel.
    pub capacity: usize,
    /// Maximum number of commands to drain after the blocking receive.
    pub batch: BatchConfig,
    /// Number of writes between automatic flushes (0 disables periodic flushing).
    pub flush_interval: usize,
    /// Optional barrier for synchronising worker startup in tests.
    pub start_barrier: Option<Arc<Barrier>>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            capacity: super::config::DEFAULT_CHANNEL_CAPACITY,
            batch: BatchConfig::new(DEFAULT_BATCH_CAPACITY).unwrap_or_default(),
            flush_interval: 1,
            start_barrier: None,
        }
    }
}

impl From<&HandlerConfig> for WorkerConfig {
    fn from(cfg: &HandlerConfig) -> Self {
        Self {
            capacity: cfg.capacity,
            batch: BatchConfig::new(DEFAULT_BATCH_CAPACITY).unwrap_or_default(),
            flush_interval: cfg.flush_interval,
            start_barrier: None,
        }
    }
}

/// Runs the receive, record, flush, and terminal-shutdown sequence on one
/// worker-owned writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecvBatchError {
    /// The command channel closed before a first command arrived.
    Disconnected,
    /// A caller supplied an invalid zero drain capacity.
    ZeroCapacity,
}

/// Receives one command and drains at most batch_capacity - 1 additional
/// commands without blocking.
fn recv_batch(
    rx: &Receiver<FileCommand>,
    batch_capacity: usize,
) -> Result<Vec<FileCommand>, RecvBatchError> {
    if batch_capacity == 0 {
        return Err(RecvBatchError::ZeroCapacity);
    }
    let first = rx
        .recv()
        .map_err(|_: RecvError| RecvBatchError::Disconnected)?;
    let mut batch = Vec::with_capacity(batch_capacity);
    batch.push(first);
    while batch.len() < batch_capacity {
        match rx.try_recv() {
            Ok(command) => batch.push(command),
            Err(_) => break,
        }
    }
    Ok(batch)
}

/// Tracks how many writes occurred and triggers periodic flushes.
pub(crate) struct FlushTracker {
    /// Counts writes since the last successful explicit or periodic flush.
    writes: usize,
    /// Flushes after this many writes; zero disables periodic flushing.
    flush_interval: usize,
}

impl FlushTracker {
    /// Starts a write counter with the configured periodic-flush interval.
    pub(crate) fn new(flush_interval: usize) -> Self {
        Self {
            writes: 0,
            flush_interval,
        }
    }

    /// Records one write and propagates any flush error produced at the new
    /// interval boundary.
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

    /// Restarts periodic-flush counting after a successful flush.
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

    /// Flushes the writer only when the configured write count is due.
    fn flush_if_due<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.should_flush() {
            writer.flush()?;
        }
        Ok(())
    }
}

/// Groups the writer, rotation state, and flush counter that are owned by one
/// worker thread.
struct WorkerState<W, R> {
    /// Buffered writer mutated only by the worker thread.
    writer: W,
    /// Rotation policy invoked immediately before each formatted record write.
    rotation: R,
    /// Tracks writes and periodic flush boundaries for this writer.
    tracker: FlushTracker,
}

impl<W, R> WorkerState<W, R>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    /// Creates worker-owned state and starts a fresh flush-counting epoch.
    fn new(writer: W, rotation: R, flush_interval: usize) -> Self {
        Self {
            writer,
            rotation,
            tracker: FlushTracker::new(flush_interval),
        }
    }

    /// Formats and writes a record on the worker thread; rotation failures are
    /// logged and do not prevent the record write from being attempted.
    fn handle_record<F>(&mut self, formatter: &F, record: FemtoLogRecord)
    where
        F: FemtoFormatter,
    {
        let message = formatter.format(&record);
        if let Err(err) = self.rotation.before_write(&mut self.writer, &message) {
            error!("FemtoFileHandler rotation error; writing record without rotating: {err}");
        }
        if let Err(err) =
            super::io_utils::write_record(&mut self.writer, &message, &mut self.tracker)
        {
            warn!("FemtoFileHandler write error: {err}");
        }
    }

    /// Flushes buffered output, resets the write counter on success, and sends
    /// the result to the waiting caller.
    fn handle_flush(&mut self, ack_tx: Sender<io::Result<()>>) {
        let flush_result = self.writer.flush();
        if let Err(err) = &flush_result {
            warn!("FemtoFileHandler flush error: {err}");
        } else {
            self.tracker.reset();
        }
        if ack_tx.send(flush_result).is_err() {
            warn!("FemtoFileHandler flush ack channel disconnected");
        }
    }

    /// Performs the terminal flush used when the command channel closes.
    fn final_flush(&mut self) {
        if let Err(err) = self.writer.flush() {
            warn!("FemtoFileHandler final flush error: {err}");
        }
    }

    /// Processes commands in receive order so records and flush acknowledgements
    /// observe one worker-owned I/O sequence.
    fn process_batch<F>(&mut self, formatter: &F, commands: Vec<FileCommand>)
    where
        F: FemtoFormatter,
    {
        for command in commands {
            match command {
                FileCommand::Record(record) => self.handle_record(formatter, *record),
                FileCommand::Flush(ack_tx) => self.handle_flush(ack_tx),
            }
        }
    }
}

/// Spawn a background worker thread for file handling.
///
/// The worker receives [`FileCommand`] values over a channel, writes formatted
/// records to `writer`, applies the `rotation` strategy before each write, and
/// flushes periodically according to `config.flush_interval`.
///
/// # Parameters
///
/// - `writer`: The underlying writer (typically a file).
/// - `formatter`: Formats log records into strings.
/// - `config`: Worker configuration (capacity, flush interval, optional start barrier).
/// - `rotation`: Strategy for rotating the log file before writes.
///
/// # Returns
///
/// A tuple containing:
/// - Command sender for enqueueing records and flush requests.
/// - Completion receiver that signals when the worker thread exits.
/// - Join handle for the worker thread.
pub fn spawn_worker<W, F, R>(
    writer: W,
    formatter: F,
    config: WorkerConfig,
    rotation: R,
) -> (Sender<FileCommand>, Receiver<()>, JoinHandle<()>)
where
    W: Write + Seek + Send + 'static,
    F: FemtoFormatter + Send + 'static,
    R: RotationStrategy<W> + Send + 'static,
{
    let WorkerConfig {
        capacity,
        batch,
        flush_interval,
        start_barrier,
    } = config;
    let (tx, rx) = bounded(capacity);
    let (done_tx, done_rx) = bounded(1);
    let handle = thread::spawn(move || {
        if let Some(b) = start_barrier {
            b.wait();
        }
        let mut state = WorkerState::new(writer, rotation, flush_interval);
        loop {
            match recv_batch(&rx, batch.capacity()) {
                Ok(commands) => state.process_batch(&formatter, commands),
                Err(RecvBatchError::Disconnected) => break,
                Err(RecvBatchError::ZeroCapacity) => {
                    error!("FemtoFileHandler batch capacity must be greater than zero");
                    break;
                }
            }
        }
        state.final_flush();
        if done_tx.send(()).is_err() {
            warn!("FemtoFileHandler done channel disconnected");
        }
    });
    (tx, done_rx, handle)
}

#[cfg(test)]
#[path = "flush_tracker_tests.rs"]
mod flush_tracker_tests;

#[cfg(test)]
#[path = "worker_batch_tests.rs"]
mod worker_batch_tests;
