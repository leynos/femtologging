//! Asynchronous file handler used by `femtologging`.
//!
//! A dedicated worker thread receives `FemtoLogRecord` values over a bounded
//! channel and writes them to disk. Python constructors map onto the Rust
//! APIs via PyO3 wrappers defined below.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Barrier},
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{bounded, Receiver, Sender};
use log::warn;
use pyo3::prelude::*;

use crate::handler::FemtoHandlerTrait;
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
};

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Determines how `FemtoFileHandler` reacts when its queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Drop new records, preserving existing ones. Current default behaviour.
    Drop,
    /// Block the caller until space becomes available.
    Block,
    /// Block up to the specified duration before giving up.
    Timeout(Duration),
}

/// Configuration options for constructing a [`FemtoFileHandler`].
#[derive(Clone, Copy)]
pub struct HandlerConfig {
    /// Bounded queue size for records waiting to be written.
    pub capacity: usize,
    /// How often the worker thread flushes the writer.
    pub flush_interval: usize,
    /// Policy to apply when the queue is full.
    pub overflow_policy: OverflowPolicy,
}

/// Configuration for `with_writer_for_test` when constructing handlers in
/// Rust unit tests.
pub struct TestConfig<W, F> {
    pub writer: W,
    pub formatter: F,
    pub capacity: usize,
    pub flush_interval: usize,
    pub overflow_policy: OverflowPolicy,
    pub start_barrier: Option<Arc<Barrier>>,
}

impl<W, F> TestConfig<W, F> {
    pub fn new(writer: W, formatter: F) -> Self {
        Self {
            writer,
            formatter,
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
            start_barrier: None,
        }
    }
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        }
    }
}

/// Configuration for the background worker thread.
struct WorkerConfig {
    capacity: usize,
    flush_interval: usize,
    start_barrier: Option<Arc<Barrier>>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_interval: 1,
            start_barrier: None,
        }
    }
}

/// Tracks how many writes occurred and triggers periodic flushes.
struct FlushTracker {
    writes: usize,
    flush_interval: usize,
}

impl FlushTracker {
    fn new(flush_interval: usize) -> Self {
        Self {
            writes: 0,
            flush_interval,
        }
    }

    fn record_write<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
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

    fn reset(&mut self) {
        self.writes = 0;
    }

    /// Determine whether the writer should flush on the current write.
    ///
    /// A flush is due when the interval is non-zero, at least one write has
    /// occurred, and the write count is a multiple of the interval.
    fn should_flush(&self) -> bool {
        self.flush_interval != 0 && self.writes > 0 && self.writes % self.flush_interval == 0
    }

    fn flush_if_due<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.should_flush() {
            writer.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod flush_tracker_tests {
    use super::*;
    use logtest::Logger;
    use rstest::*;
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

/// Handler that writes formatted log records to a file on a background thread.
enum FileCommand {
    Record(FemtoLogRecord),
    Flush(Sender<()>),
}

#[pyclass]
pub struct FemtoFileHandler {
    tx: Option<Sender<FileCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
    overflow_policy: OverflowPolicy,
}

#[pymethods]
impl FemtoFileHandler {
    /// Python constructor mirroring `new` but raising `OSError` on failure.
    #[new]
    fn py_new(path: String) -> PyResult<Self> {
        Self::new(path).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Construct a handler with a caller-specified queue size.
    #[staticmethod]
    #[pyo3(name = "with_capacity")]
    fn py_with_capacity(path: String, capacity: usize) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, None, OverflowPolicy::Drop)
    }

    /// Create a blocking handler that waits when the queue is full.
    #[staticmethod]
    #[pyo3(name = "with_capacity_blocking")]
    fn py_with_capacity_blocking(path: String, capacity: usize) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, None, OverflowPolicy::Block)
    }

    /// Create a timeout-based handler. `timeout_ms` specifies how long to wait for space.
    #[staticmethod]
    #[pyo3(name = "with_capacity_timeout")]
    fn py_with_capacity_timeout(path: String, capacity: usize, timeout_ms: u64) -> PyResult<Self> {
        Self::build_py_handler(
            path,
            capacity,
            None,
            OverflowPolicy::Timeout(Duration::from_millis(timeout_ms)),
        )
    }

    /// Create a handler with a custom flush interval.
    ///
    /// `flush_interval` controls how often the worker thread flushes the
    /// underlying file. A value of `0` disables periodic flushing and only
    /// flushes when the handler shuts down.
    #[staticmethod]
    #[pyo3(name = "with_capacity_flush")]
    fn py_with_capacity_flush(
        path: String,
        capacity: usize,
        flush_interval: usize,
    ) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, Some(flush_interval), OverflowPolicy::Drop)
    }

    /// Blocking variant of `with_capacity_flush`.
    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_blocking")]
    fn py_with_capacity_flush_blocking(
        path: String,
        capacity: usize,
        flush_interval: usize,
    ) -> PyResult<Self> {
        Self::build_py_handler(path, capacity, Some(flush_interval), OverflowPolicy::Block)
    }

    /// Timeout variant of `with_capacity_flush`.
    #[staticmethod]
    #[pyo3(name = "with_capacity_flush_timeout")]
    fn py_with_capacity_flush_timeout(
        path: String,
        capacity: usize,
        flush_interval: usize,
        timeout_ms: u64,
    ) -> PyResult<Self> {
        Self::build_py_handler(
            path,
            capacity,
            Some(flush_interval),
            OverflowPolicy::Timeout(Duration::from_millis(timeout_ms)),
        )
    }

    /// Dispatch a log record created from the provided parameters.
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) {
        <Self as FemtoHandlerTrait>::handle(self, FemtoLogRecord::new(logger, level, message));
    }

    /// Flush pending log records without shutting down the worker thread.
    #[pyo3(name = "flush")]
    fn py_flush(&self) -> bool {
        self.flush()
    }

    /// Close the handler and wait for the worker thread to finish.
    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
    }
}
impl FemtoFileHandler {
    /// Helper used by the Python constructors to build a handler while
    /// translating I/O errors into `OSError` for Python callers.
    fn build_py_handler(
        path: String,
        capacity: usize,
        flush_interval: Option<usize>,
        overflow_policy: OverflowPolicy,
    ) -> PyResult<Self> {
        Self::handle_io_result(Self::create_with_policy(
            path,
            capacity,
            flush_interval,
            overflow_policy,
        ))
    }

    /// Convenience constructor using the default formatter and queue capacity.
    /// Spawn the worker thread that processes file commands.
    fn spawn_worker<W, F>(
        writer: W,
        formatter: F,
        config: WorkerConfig,
    ) -> (Sender<FileCommand>, Receiver<()>, JoinHandle<()>)
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let WorkerConfig {
            capacity,
            flush_interval,
            start_barrier,
            ..
        } = config;
        let (tx, rx) = bounded(capacity);
        let (done_tx, done_rx) = bounded(1);
        let handle = thread::spawn(move || {
            if let Some(b) = start_barrier {
                b.wait();
            }
            let mut writer = writer;
            let formatter = formatter;
            let mut tracker = FlushTracker::new(flush_interval);
            for cmd in rx {
                match cmd {
                    FileCommand::Record(record) => {
                        if let Err(e) =
                            Self::write_record(&mut writer, &formatter, record, &mut tracker)
                        {
                            warn!("FemtoFileHandler write error: {e}");
                        }
                    }
                    FileCommand::Flush(ack) => {
                        if writer.flush().is_err() {
                            warn!("FemtoFileHandler flush error");
                        }
                        tracker.reset();
                        let _ = ack.send(());
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

    fn build_config(
        capacity: usize,
        flush_interval: Option<usize>,
        overflow_policy: OverflowPolicy,
    ) -> HandlerConfig {
        let defaults = HandlerConfig::default();
        HandlerConfig {
            capacity,
            flush_interval: flush_interval.unwrap_or(defaults.flush_interval),
            overflow_policy,
        }
    }

    fn handle_io_result(result: io::Result<Self>) -> PyResult<Self> {
        result.map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    fn create_with_policy<P: AsRef<Path>>(
        path: P,
        capacity: usize,
        flush_interval: Option<usize>,
        overflow_policy: OverflowPolicy,
    ) -> io::Result<Self> {
        let cfg = Self::build_config(capacity, flush_interval, overflow_policy);
        Self::with_capacity_flush_policy(path, DefaultFormatter, cfg)
    }

    /// Write a single log record to the provided writer.
    fn write_record<W, F>(
        writer: &mut W,
        formatter: &F,
        record: FemtoLogRecord,
        flush_tracker: &mut FlushTracker,
    ) -> io::Result<()>
    where
        W: Write,
        F: FemtoFormatter,
    {
        let msg = formatter.format(&record);

        writeln!(writer, "{msg}")?;

        flush_tracker.record_write(writer)
    }
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::with_capacity(path, DefaultFormatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new handler with a custom formatter and bounded queue size.
    ///
    /// `capacity` controls the length of the internal channel used to pass
    /// records to the worker thread. When full, new records are dropped.
    pub fn with_capacity<P, F>(path: P, formatter: F, capacity: usize) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let cfg = Self::build_config(capacity, None, OverflowPolicy::Drop);
        Self::with_capacity_flush_policy(path, formatter, cfg)
    }

    /// Create a new handler with custom capacity and flush interval.
    ///
    /// `flush_interval` determines how many records are written before the
    /// worker thread flushes the file. A value of `0` disables periodic flushes
    /// and only flushes on shutdown.
    pub fn with_capacity_flush_interval<P, F>(
        path: P,
        formatter: F,
        capacity: usize,
        flush_interval: usize,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let cfg = Self::build_config(capacity, Some(flush_interval), OverflowPolicy::Drop);
        Self::with_capacity_flush_policy(path, formatter, cfg)
    }

    /// Create a handler with explicit overflow policy.
    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(file, formatter, config))
    }

    /// Build a handler using an already opened `File` and custom formatter.
    ///
    /// This is primarily used by `with_capacity` after opening the file.
    fn from_file<F>(file: File, formatter: F, config: HandlerConfig) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        let worker_cfg = WorkerConfig {
            capacity: config.capacity,
            flush_interval: config.flush_interval,
            start_barrier: None,
        };
        let (tx, done_rx, handle) = Self::spawn_worker(file, formatter, worker_cfg);
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy: config.overflow_policy,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        if let Some(tx) = &self.tx {
            let (ack_tx, ack_rx) = bounded(1);
            if tx.send(FileCommand::Flush(ack_tx)).is_err() {
                return false;
            }
            return ack_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        }
        false
    }

    /// Close the handler and wait for the worker thread to exit.
    pub fn close(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            if self.done_rx.recv_timeout(Duration::from_secs(1)).is_err() {
                warn!("FemtoFileHandler: worker thread did not shut down within 1s");
                return;
            }
            if handle.join().is_err() {
                warn!("FemtoFileHandler: worker thread panicked");
            }
        }
    }
}

impl FemtoHandlerTrait for FemtoFileHandler {
    /// Send a `FemtoLogRecord` to the worker thread.
    ///
    /// Behaviour depends on the overflow policy:
    /// - `Drop`: never blocks and discards the record if the queue is full.
    /// - `Block`: waits until space becomes available.
    /// - `Timeout`: waits for the configured duration before giving up.
    fn handle(&self, record: FemtoLogRecord) {
        if let Some(tx) = &self.tx {
            match self.overflow_policy {
                OverflowPolicy::Drop => {
                    if tx.try_send(FileCommand::Record(record)).is_err() {
                        warn!(
                            "FemtoFileHandler (Drop): queue full or shutting down, dropping record"
                        );
                    }
                }
                OverflowPolicy::Block => {
                    if tx.send(FileCommand::Record(record)).is_err() {
                        warn!(
                            "FemtoFileHandler (Block): queue full or shutting down, dropping record"
                        );
                    }
                }
                OverflowPolicy::Timeout(dur) => {
                    if tx.send_timeout(FileCommand::Record(record), dur).is_err() {
                        warn!(
                            "FemtoFileHandler (Timeout): timed out waiting for queue, dropping record"
                        );
                    }
                }
            }
        } else {
            warn!("FemtoFileHandler: handle called after close");
        }
    }
}

impl Drop for FemtoFileHandler {
    /// Wait for the worker thread to finish processing remaining records.
    ///
    /// If the thread does not confirm shutdown within one second, a warning is
    /// logged and the handler drops without joining the thread.
    fn drop(&mut self) {
        self.close();
    }
}

impl FemtoFileHandler {
    /// Construct a handler from an arbitrary writer for testing.
    #[cfg(feature = "test-util")]
    pub fn with_writer_for_test<W, F>(config: TestConfig<W, F>) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let TestConfig {
            writer,
            formatter,
            capacity,
            flush_interval,
            overflow_policy,
            start_barrier,
        } = config;
        let worker_cfg = WorkerConfig {
            capacity,
            flush_interval,
            start_barrier,
        };
        let (tx, done_rx, handle) = Self::spawn_worker(writer, formatter, worker_cfg);
        Self {
            tx: Some(tx),
            handle: Some(handle),
            done_rx,
            overflow_policy,
        }
    }
}
