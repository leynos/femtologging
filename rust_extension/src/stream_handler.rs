//! Stream-based logging handler implementation.
//!
//! This module defines `FemtoStreamHandler`, which formats log records and
//! writes them to a stream on a background thread. The handler forwards log
//! records and flush commands over a bounded channel so the producer never
//! blocks on I/O. The handler supports explicit flushing to ensure all pending
//! records are written.
// PyO3 adds an implicit `py` argument to generated wrappers; allow higher
// parameter counts for Python-facing methods.
#![allow(clippy::too_many_arguments)]

use std::{
    io::{self, Write},
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use log::warn;
use parking_lot::Mutex;
use pyo3::prelude::*;
use std::any::Any;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
    rate_limited_warner::{RateLimitedWarner, DEFAULT_WARN_INTERVAL},
};

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Configuration for constructing a [`FemtoStreamHandler`].
pub struct HandlerConfig {
    pub capacity: usize,
    pub flush_timeout: Duration,
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
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = timeout;
        self
    }

    #[cfg(feature = "test-util")]
    pub fn with_warner(mut self, warner: RateLimitedWarner) -> Self {
        self.warner = warner;
        self
    }
}

/// Handler that writes formatted log records to an `io::Write` stream.
///
/// Each instance owns a background thread which receives records and flush
/// commands via a channel and writes them to the provided stream. The writer
/// and formatter are moved into that thread so the caller never locks or
/// blocks. The handler supports explicit flushing to ensure all queued records
/// are written. Flush operations wait up to `flush_timeout` for the worker
/// thread to confirm completion.
enum StreamCommand {
    Record(FemtoLogRecord),
    Flush(Sender<()>),
}

fn flush_with_warning<W: Write>(writer: &mut W) {
    if writer.flush().is_err() {
        warn!("FemtoStreamHandler flush error");
    }
}

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

fn handle_flush_command<W: Write>(writer: &mut W, ack: Sender<()>) {
    flush_with_warning(writer);
    let _ = ack.send(());
}

fn run_stream_worker<W, F>(
    rx: Receiver<StreamCommand>,
    mut writer: W,
    formatter: F,
    done_tx: Sender<()>,
) where
    W: Write,
    F: FemtoFormatter,
{
    for cmd in rx {
        match cmd {
            StreamCommand::Record(record) => handle_record_command(&mut writer, &formatter, record),
            StreamCommand::Flush(ack) => handle_flush_command(&mut writer, ack),
        }
    }
    flush_with_warning(&mut writer);
    let _ = done_tx.send(());
}

#[pyclass]
pub struct FemtoStreamHandler {
    tx: Option<Sender<StreamCommand>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    done_rx: Mutex<Receiver<()>>,
    /// Tracks dropped records and rate-limits warnings.
    warner: RateLimitedWarner,
    /// Timeout for flush operations.
    flush_timeout: Duration,
}

// PyO3 adds an implicit `py` argument to generated wrappers; keep the Python
// API stable and silence the argument-count lint locally.
#[allow(clippy::too_many_arguments)]
#[pymethods]
impl FemtoStreamHandler {
    #[new]
    fn py_new() -> Self {
        Self::stderr()
    }

    #[staticmethod]
    #[pyo3(name = "stdout")]
    fn py_stdout() -> Self {
        Self::stdout()
    }

    #[staticmethod]
    #[pyo3(name = "stderr")]
    fn py_stderr() -> Self {
        Self::stderr()
    }

    /// Dispatch a log record to the handler's worker thread.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        <Self as FemtoHandlerTrait>::handle(self, FemtoLogRecord::new(logger, level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush pending log records without shutting down the worker thread.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush command within the
    ///     timeout (default: 1 second).
    ///     ``False`` when the handler has already been closed, the command
    ///     cannot be delivered to the worker, or the worker fails to
    ///     acknowledge before the timeout elapses.
    ///
    /// Examples
    /// --------
    /// >>> handler.flush()
    /// True
    /// >>> handler.close()
    /// >>> handler.flush()
    /// False
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

    #[cfg(feature = "test-util")]
    pub fn with_test_config<W, F>(writer: W, formatter: F, config: HandlerConfig) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        Self::with_config(writer, formatter, config)
    }

    fn with_config<W, F>(writer: W, formatter: F, config: HandlerConfig) -> Self
    where
        W: Write + Send + 'static,
        F: FemtoFormatter + Send + 'static,
    {
        let (tx, rx) = bounded(config.capacity);
        let (done_tx, done_rx) = bounded(1);
        let handle = thread::spawn(move || run_stream_worker(rx, writer, formatter, done_tx));

        Self {
            tx: Some(tx),
            handle: Mutex::new(Some(handle)),
            done_rx: Mutex::new(done_rx),
            warner: config.warner,
            flush_timeout: config.flush_timeout,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        <Self as FemtoHandlerTrait>::flush(self)
    }

    /// Close the handler and wait for the worker thread to exit.
    pub fn close(&mut self) {
        self.tx.take();
        let handle = { self.handle.lock().take() };
        if let Some(handle) = handle {
            let done_rx = self.done_rx.lock().clone();
            if done_rx.recv_timeout(self.flush_timeout).is_err() {
                warn!(
                    "FemtoStreamHandler: worker thread did not shut down within {:?}",
                    self.flush_timeout
                );
                return;
            }
            if handle.join().is_err() {
                warn!("FemtoStreamHandler: worker thread panicked");
            }
        }
    }
}

impl FemtoHandlerTrait for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = &self.tx else {
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
        match &self.tx {
            Some(tx) => {
                self.warner.flush(|count| {
                    warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
                });
                let (ack_tx, ack_rx) = bounded(1);
                if tx
                    .send_timeout(StreamCommand::Flush(ack_tx), self.flush_timeout)
                    .is_err()
                {
                    return false;
                }
                ack_rx.recv_timeout(self.flush_timeout).is_ok()
            }
            None => false,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        self.close();
    }
}
