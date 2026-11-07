//! Stream-based logging handler implementation.
//!
//! This module defines `FemtoStreamHandler`, which formats log records and
//! writes them to a stream on a background thread. The handler forwards log
//! records and flush commands over a bounded channel so the producer never
//! blocks on I/O. The handler supports explicit flushing to ensure all pending
//! records are written.

mod config;

mod shutdown;

use std::{
    any::Any,
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};

use log::warn;
use pyo3::prelude::*;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::shutdown::{CompleteOnDrop, ShutdownCompletion};
use crate::sync::{JoinHandle, Mutex, Receiver, Sender, TrySendError, bounded, spawn};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
};

pub use config::{DEFAULT_CHANNEL_CAPACITY, HandlerConfig};
use shutdown::StreamShutdown;

/// Commands the worker thread takes from the queue, in order.
#[expect(
    clippy::large_enum_variant,
    reason = "Record variant is the hot path; wrapping in Box would add indirection for no benefit"
)]
enum StreamCommand {
    /// Owned record consumed by the worker in queue order.
    Record(FemtoLogRecord),
    /// Flush request carrying the worker's I/O result acknowledgement channel.
    Flush(Sender<io::Result<()>>),
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
fn handle_flush_command<W: Write>(writer: &mut W, ack: Sender<io::Result<()>>) {
    let flush_result = writer.flush();
    if flush_result.is_err() {
        warn!("FemtoStreamHandler flush error");
    }
    let _ = ack.send(flush_result);
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
    // Declared first so it drops last: the writer and formatter are dropped
    // before closers are told the worker stopped, and a panic still signals
    // them. The work runs inside a closure so its captures are dropped when
    // the call ends, including while unwinding; a formatter whose destructor
    // blocks must not let a closer's wait outlive the flush timeout.
    let _signal = CompleteOnDrop::new(completion);
    let work = move || {
        let mut writer = writer;
        for cmd in rx {
            match cmd {
                StreamCommand::Record(record) => {
                    handle_record_command(&mut writer, &formatter, record);
                }
                StreamCommand::Flush(ack) => handle_flush_command(&mut writer, ack),
            }
        }
        flush_with_warning(&mut writer);
    };
    // No closer joins this thread (a join would wait for thread-local
    // destructors that user code can make block), so the worker reports its
    // own panic instead of leaving the join result unread.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
        warn!("FemtoStreamHandler: worker thread panicked");
    }
}

/// Writes formatted log records to a stream (stderr by default) on a background thread, so
/// the caller never waits for the write. `flush` bounds its whole wait by `flush_timeout`.
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
    /// run on the worker thread. A single deadline, derived from the handler's
    /// flush timeout, bounds the whole call: queueing the flush command and
    /// waiting for the worker's acknowledgement share it. When it expires,
    /// ``False`` is returned and the worker may still write later.
    #[pyo3(name = "flush")]
    fn py_flush(&self, py: Python<'_>) -> bool {
        py.detach(|| self.flush())
    }

    /// Close the handler and wait for the worker thread to finish.
    ///
    /// The handler stops accepting records, drains the records already queued,
    /// and waits for the worker to exit. The GIL is released while waiting, so
    /// a Python callable formatter can run to completion on the worker thread.
    /// Only a shared PyO3 borrow is needed, so a close can run beside a
    /// detached flush call.
    ///
    /// The wait is bounded by the handler's flush timeout. If it expires,
    /// close returns while the worker may still write to the stream. On
    /// success the worker has written and flushed its queued records, but the
    /// thread is detached rather than joined, because a join would also wait
    /// for thread-local destructors that user code can make block.
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
    pub fn flush(&self) -> bool {
        <Self as FemtoHandlerTrait>::flush(self)
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

impl FemtoHandlerTrait for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        // Clone under the lock and release it before sending: a blocking send
        // must not hold the lock that close needs to take the sender.
        let Some(tx) = self.tx.lock().clone() else {
            self.warner.record_drop();
            self.warner.warn_if_due(|count| {
                warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
            });
            return Err(HandlerError::Closed);
        };
        match tx.try_send(StreamCommand::Record(record)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.warner.record_drop();
                self.warner.warn_if_due(|count| {
                    warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
                });
                Err(HandlerError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.warner.record_drop();
                self.warner.warn_if_due(|count| {
                    warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
                });
                Err(HandlerError::Closed)
            }
        }
    }

    fn flush(&self) -> bool {
        // Bind first: a guard in the `match` scrutinee lives through the send.
        let tx = self.tx.lock().clone();
        match tx {
            Some(tx) => {
                self.warner.flush(|count| {
                    warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
                });
                let (ack_tx, ack_rx) = bounded(1);
                // One deadline covers both waits. The channel send can block
                // when the queue is full and the acknowledgement can take a
                // further interval, so applying the timeout to each in turn
                // would let a flush wait nearly twice the configured limit.
                let deadline = Instant::now() + self.flush_timeout;
                if tx
                    .send_timeout(StreamCommand::Flush(ack_tx), self.flush_timeout)
                    .is_err()
                {
                    return false;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                matches!(ack_rx.recv_timeout(remaining), Ok(Ok(())))
            }
            None => false,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
