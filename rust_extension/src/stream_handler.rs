//! Stream-based logging handler implementation.
//!
//! This module defines `FemtoStreamHandler`, which formats log records and
//! writes them to a stream on a background thread. The handler forwards log
//! records and flush commands over a bounded channel so the producer never
//! blocks on I/O. The handler supports explicit flushing to ensure all pending
//! records are written.
mod shutdown;

mod trait_impl;

use std::{
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};

use log::warn;
use pyo3::prelude::*;

use crate::handler::{FemtoHandlerTrait, capped_deadline, deadline_after, remaining};
use crate::shutdown::{CompleteOnDrop, ShutdownCompletion};
use crate::sync::{JoinHandle, Mutex, Receiver, Sender, bounded, spawn};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
    rate_limited_warner::{DEFAULT_WARN_INTERVAL, RateLimitedWarner},
};

use shutdown::StreamShutdown;

/// Default queue capacity used when a stream builder does not provide one.
const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Configuration for constructing a [`FemtoStreamHandler`].
pub struct HandlerConfig {
    /// Bounded queue size, in commands, between callers and the worker. Records arriving
    /// while the queue is full are dropped.
    pub capacity: usize,
    /// How long `flush` waits to enqueue its command and, separately, for the worker's acknowledgement.
    pub flush_timeout: Duration,
    /// Rate limiter that throttles warnings about dropped records.
    pub warner: RateLimitedWarner,
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_timeout: Duration::from_secs(1),
            warner: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
        }
    }
}

impl HandlerConfig {
    /// Set the worker queue capacity, in commands.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Set how long each of `flush`'s two waits (enqueue, acknowledgement) may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = timeout;
        self
    }

    /// Replace the rate limiter used for drop warnings. Available with the `test-util` feature.
    #[cfg(feature = "test-util")]
    pub fn with_warner(mut self, warner: RateLimitedWarner) -> Self {
        self.warner = warner;
        self
    }
}

/// Commands the worker thread takes from the queue, in order.
#[expect(
    clippy::large_enum_variant,
    reason = "Record variant is the hot path; wrapping in Box would add indirection for no benefit"
)]
enum StreamCommand {
    /// Owned record consumed by the worker in queue order.
    Record(FemtoLogRecord),
    /// Flush request carrying its acknowledgement and deferred drop warning.
    Flush(Sender<io::Result<()>>, Option<u64>),
}

/// Flushes the writer during worker shutdown and logs, but does not propagate, errors.
fn flush_with_warning<W: Write>(writer: &mut W) {
    if writer.flush().is_err() {
        warn!("FemtoStreamHandler flush error");
    }
}

/// Formats one queued record, writes a newline, and flushes the worker-owned writer.
fn handle_record_command<W, F>(writer: &mut W, formatter: &F, record: FemtoLogRecord)
where
    W: Write,
    F: FemtoFormatter,
{
    let msg = formatter.format(&record);
    if writeln!(writer, "{msg}")
        .and_then(|_| writer.flush())
        .is_err()
    {
        warn!("FemtoStreamHandler write error");
    }
}

/// Flushes the writer and sends the exact I/O result to the waiting producer.
fn handle_flush_command<W: Write>(
    writer: &mut W,
    ack: Sender<io::Result<()>>,
    pending_drops: Option<u64>,
) {
    let flush_result = writer.flush();
    let flush_failed = flush_result.is_err();
    let _ = ack.send(flush_result);
    if flush_failed {
        warn!("FemtoStreamHandler flush error");
    }
    if let Some(count) = pending_drops {
        warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
    }
}

/// Owns the writer and formatter on the background thread until the queue closes.
fn run_stream_worker<W, F>(
    rx: Receiver<StreamCommand>,
    writer: W,
    formatter: F,
    completion: Arc<ShutdownCompletion>,
) where
    W: Write,
    F: FemtoFormatter,
{
    // Declared first so it drops last: the writer is dropped before closers
    // are told the worker stopped, and a panic still signals them.
    let _signal = CompleteOnDrop::new(completion);
    let mut writer = writer;
    for cmd in rx {
        match cmd {
            StreamCommand::Record(record) => handle_record_command(&mut writer, &formatter, record),
            StreamCommand::Flush(ack, pending_drops) => {
                handle_flush_command(&mut writer, ack, pending_drops);
            }
        }
    }
    flush_with_warning(&mut writer);
}

/// Writes formatted log records to a stream (stderr by default) on a background thread, so
/// the caller never waits for the write. `flush` bounds each of its two waits by `flush_timeout`.
#[pyclass]
pub struct FemtoStreamHandler {
    /// Queue sender taken during close; shared so no closer needs an exclusive borrow.
    tx: Mutex<Option<Sender<StreamCommand>>>,
    /// Worker join handle protected so close and drop cannot join twice.
    handle: Mutex<Option<JoinHandle<()>>>,
    /// Completion broadcast every closing thread waits on, so a closer that
    /// does not own the join handle still observes worker shutdown.
    completion: Arc<ShutdownCompletion>,
    /// Tracks dropped records and rate-limits warnings.
    warner: RateLimitedWarner,
    /// Timeout for flush operations.
    flush_timeout: Duration,
}

#[pymethods]
impl FemtoStreamHandler {
    /// Constructs the Python-facing default handler, which writes to stderr.
    #[new]
    fn py_new() -> Self {
        Self::stderr()
    }

    /// Constructs a handler writing to process stdout with the default formatter.
    #[staticmethod]
    #[pyo3(name = "stdout")]
    fn py_stdout() -> Self {
        Self::stdout()
    }

    /// Constructs a handler writing to process stderr with the default formatter.
    #[staticmethod]
    #[pyo3(name = "stderr")]
    fn py_stderr() -> Self {
        Self::stderr()
    }

    /// Parses the Python level, creates a record, and queues it for the worker.
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        <Self as FemtoHandlerTrait>::handle(
            self,
            FemtoLogRecord::new(logger, parsed_level, message),
        )
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush pending log records without shutting down the worker thread.
    ///
    /// The flush timeout is configurable (default: 1 second).
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` only when the worker acknowledges the flush with
    ///     ``Ok(())`` within the configured timeout.
    ///     ``False`` when the handler has already been closed, the
    ///     internal channel to the worker has been dropped, the worker
    ///     acknowledges ``Err(io::Error)``, or the acknowledgement does not
    ///     arrive before the timeout elapses.
    ///
    /// Examples
    /// --------
    /// >>> handler.flush()
    /// True
    /// >>> handler.close()
    /// >>> handler.flush()
    /// False
    ///
    /// The GIL is released while waiting so a Python callable formatter can
    /// run on the worker thread.
    #[pyo3(name = "flush")]
    fn py_flush(&self, py: Python<'_>) -> bool {
        py.detach(|| self.flush())
    }

    /// Close the handler and wait for the worker thread to finish.
    ///
    /// The GIL is released while waiting, and only a shared PyO3 borrow is
    /// needed, so a close can run beside a detached flush call.
    #[pyo3(name = "close")]
    fn py_close(slf: Py<Self>, py: Python<'_>) {
        let shutdown = {
            let handler = slf.borrow(py);
            handler.take_shutdown()
        };
        py.detach(move || shutdown.wait());
    }
}

impl FemtoStreamHandler {
    /// Create a new handler writing to `stdout` with a `DefaultFormatter`.
    pub fn stdout() -> Self {
        Self::new(io::stdout(), DefaultFormatter)
    }

    /// Create a new handler writing to `stderr` with a `DefaultFormatter`.
    pub fn stderr() -> Self {
        Self::new(io::stderr(), DefaultFormatter)
    }

    /// Create a new handler from an arbitrary writer and formatter using the default capacity.
    pub fn new<W, F>(writer: W, formatter: F) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_capacity(writer, formatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new handler with a custom channel capacity.
    pub fn with_capacity<W, F>(writer: W, formatter: F, capacity: usize) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_capacity_timeout(writer, formatter, capacity, Duration::from_secs(1))
    }

    /// Create a new handler with custom capacity and flush timeout.
    pub fn with_capacity_timeout<W, F>(
        writer: W,
        formatter: F,
        capacity: usize,
        flush_timeout: Duration,
    ) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_config(
            writer,
            formatter,
            HandlerConfig::default()
                .with_capacity(capacity)
                .with_timeout(flush_timeout),
        )
    }

    /// Build a handler from an explicit `HandlerConfig`, for tests that control the queue,
    /// flush timeout, or rate limiter.
    #[cfg(feature = "test-util")]
    pub fn with_test_config<W, F>(writer: W, formatter: F, config: HandlerConfig) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_config(writer, formatter, config)
    }

    /// Builds a worker and moves the writer and formatter into it; capacity and timeout govern acknowledgements.
    fn with_config<W, F>(writer: W, formatter: F, config: HandlerConfig) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, rx) = bounded(config.capacity);
        let completion = ShutdownCompletion::new();
        let worker_completion = Arc::clone(&completion);
        let handle = spawn(move || run_stream_worker(rx, writer, formatter, worker_completion));

        Self {
            tx: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
            completion,
            warner: config.warner,
            flush_timeout: config.flush_timeout,
        }
    }

    /// Flush any pending log records.
    ///
    /// Python callers release the interpreter while waiting for the handler's
    /// worker acknowledgement, allowing Python formatters to run there.
    pub fn flush(&self) -> bool {
        let Some(deadline) = deadline_after(Instant::now(), self.flush_timeout) else {
            return false;
        };
        self.flush_by(deadline)
    }

    /// Flush any pending log records, waiting no longer than `deadline`.
    ///
    /// The wait also stops at this handler's configured `flush_timeout`, so a
    /// bare [`FemtoStreamHandler::flush`] keeps the bound it documents and a
    /// barrier flush cannot outlive the deadline it was given.
    fn flush_by(&self, deadline: Instant) -> bool {
        // Whichever comes first — the caller's deadline or this handler's own
        // budget — bounds the flush as a whole. Giving each wait the budget
        // afresh let one flush spend it twice over.
        let effective = capped_deadline(deadline, Instant::now(), self.flush_timeout);
        // Bind first: a guard in the `match` scrutinee lives through the send.
        let tx = self.tx.lock().clone();
        match tx {
            Some(tx) => {
                let pending_drops = self.warner.take_due();
                let (ack_tx, ack_rx) = bounded(1);
                if tx
                    .send_timeout(
                        StreamCommand::Flush(ack_tx, pending_drops),
                        remaining(effective, Instant::now()),
                    )
                    .is_err()
                {
                    if let Some(count) = pending_drops {
                        self.warner.restore_drops(count);
                    }
                    return false;
                }
                matches!(
                    ack_rx.recv_timeout(remaining(effective, Instant::now())),
                    Ok(Ok(()))
                )
            }
            None => false,
        }
    }

    /// Close the handler and wait for the worker thread to exit.
    ///
    /// Concurrent callers all wait for the same completion.
    pub fn close(&self) {
        self.take_shutdown().wait();
    }

    /// Marks the handler closed and transfers worker state to a shutdown
    /// operation that can wait after the Python object borrow is released.
    fn take_shutdown(&self) -> StreamShutdown {
        let tx = self.tx.lock().take();
        drop(tx);
        let handle = self.handle.lock().take();
        let completion = Arc::clone(&self.completion);
        StreamShutdown::new(handle, completion, self.flush_timeout)
    }
}
