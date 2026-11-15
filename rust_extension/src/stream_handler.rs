//! Stream-based logging handler implementation.
//!
//! This module defines `FemtoStreamHandler`, which formats log records and
//! writes them to a stream on a background thread. The handler forwards log
//! records and flush commands over a bounded channel so the producer never
//! blocks on I/O. The handler supports explicit flushing to ensure all pending
//! records are written.

mod config;

mod shutdown;

mod trait_impl;

mod waiter;

mod worker;

use std::{
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};

use pyo3::prelude::*;

use crate::handler::{FemtoHandlerTrait, FlushCap, FlushContext, deadline_after};
use crate::shutdown::ShutdownCompletion;
use crate::sync::{JoinHandle, Mutex, Sender, bounded, spawn};
use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    log_record::FemtoLogRecord,
    monotonic_clock::SharedClock,
    rate_limited_warner::RateLimitedWarner,
};

pub use config::{DEFAULT_CHANNEL_CAPACITY, HandlerConfig};
use shutdown::StreamShutdown;
use waiter::WaitGuard;
#[cfg(feature = "test-util")]
pub use waiter::waiting_count;
use worker::run_stream_worker;

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

/// Writes formatted log records to a stream (stderr by default) on a background thread, so
/// the caller never waits for the write. `flush` spends at most `flush_timeout` in total
/// across enqueueing its command and waiting for the worker's acknowledgement.
#[pyclass]
pub struct FemtoStreamHandler {
    /// Queue sender taken during close; shared so no closer needs an exclusive borrow.
    tx: Mutex<Option<Sender<StreamCommand>>>,
    /// Worker handle taken by the closer that detaches the thread; dropping it
    /// twice is harmless.
    handle: Mutex<Option<JoinHandle<()>>>,
    /// Completion broadcast every closing thread waits on, so a closer that
    /// does not own the join handle still observes worker shutdown.
    completion: Arc<ShutdownCompletion>,
    /// Tracks dropped records and rate-limits warnings.
    warner: RateLimitedWarner,
    /// Timeout for flush operations.
    flush_timeout: Duration,
    /// Clock the flush deadline is measured against, injected through
    /// `HandlerConfig` so a test can script the deadline alone.
    clock: SharedClock,
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
    ///     internal channel to the worker has been dropped, the flush
    ///     command cannot be queued before the timeout elapses, the worker
    ///     acknowledges ``Err(io::Error)``, or the acknowledgement does not
    ///     arrive before the timeout elapses. The value does not identify
    ///     which of these happened.
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

    /// Close the handler and wait for the worker to stop.
    ///
    /// The handler stops accepting records, and the caller waits for the worker
    /// to raise its completion signal, which happens after the worker's body
    /// ends and its writer and formatter have been dropped. The GIL is released
    /// while waiting, so a Python callable formatter can run to completion on
    /// the worker thread. Only a shared PyO3 borrow is needed, so a close can
    /// run beside a detached flush call.
    ///
    /// The wait is bounded by the handler's flush timeout. If it expires, close
    /// returns while the worker may still be writing, and no return value
    /// distinguishes that case from a completed one. Completion is not proof of
    /// delivery either: a worker panic still raises it with records queued, and
    /// write errors are logged rather than returned. The thread is detached
    /// rather than joined, because a join would also wait for thread-local
    /// destructors that user code can make block.
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
            clock: config.clock,
        }
    }

    /// Flush any pending log records.
    ///
    /// Python callers release the interpreter while waiting for the handler's
    /// worker acknowledgement, allowing Python formatters to run there.
    pub fn flush(&self) -> bool {
        let now = self.clock.now();
        let Some(deadline) = deadline_after(now, self.flush_timeout) else {
            return false;
        };
        self.flush_by(&FlushContext::with_clock(
            now,
            Some(deadline),
            Arc::clone(&self.clock),
        ))
    }

    /// Flush any pending log records, waiting no longer than the context's
    /// deadline.
    ///
    /// The wait also stops at this handler's configured `flush_timeout`, so a
    /// bare `flush()` keeps the bound it documents and a barrier flush cannot
    /// outlive the deadline it was given. Time is sampled from the context, so
    /// a barrier test controls this handler's deadline transition as well as
    /// the logger's.
    fn flush_by(&self, context: &FlushContext) -> bool {
        let Some(deadline) = context.deadline() else {
            return false;
        };
        self.flush_by_at(deadline, &mut || context.now())
    }

    /// Flush against `deadline`, sampling each queue wait from `now`.
    ///
    /// Production passes [`Instant::now`] here; tests pass a scripted source,
    /// so a stalled worker queue can be shown to consume exactly the budget
    /// the caller supplied rather than a fresh copy for each wait.
    fn flush_by_at(&self, deadline: Instant, now: &mut dyn FnMut() -> Instant) -> bool {
        // The helper below runs both bounded queue waits, so the whole call is
        // inside them from here. The marker lets a shutdown test release a
        // gated formatter only once this thread has reached its wait, instead
        // of inferring that from elapsed time.
        let _waiting = WaitGuard::enter();
        // Cap first, then read the sender: the guard is taken and released
        // inside the closure, so no lock outlives the deadline sample.
        let mut cap = FlushCap::new(deadline, self.flush_timeout, now);
        let outcome = self.warner.flush_handler(
            &mut cap,
            || self.tx.lock().clone(),
            |tx, pending_drops, budget| {
                let (ack_tx, ack_rx) = bounded(1);
                tx.send_timeout(StreamCommand::Flush(ack_tx, pending_drops), budget)
                    .ok()?;
                Some(move |budget| ack_rx.recv_timeout(budget).ok())
            },
        );
        matches!(outcome, Some(Ok(Ok(()))))
    }

    /// Close the handler and wait for the worker to finish its queued work.
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
