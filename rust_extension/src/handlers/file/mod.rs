//! Public API for the file-based logging handler.
//!
//! `FemtoFileHandler` spawns a dedicated worker thread that writes formatted
//! log records to disk. Configuration types and the worker implementation live
//! in submodules and are re-exported here for external use.
//!
//! Construct the handler with [`FemtoFileHandler::new`] for defaults,
//! [`FemtoFileHandler::with_capacity`] to tune the queue size, or
//! [`FemtoFileHandler::with_capacity_flush_policy`] for full control in Rust.
//! Python callers customize these options via keyword arguments to
//! `FemtoFileHandler`.
//!
//! The flush interval must be greater than zero. A value of 1 flushes on every
//! record.
mod builder_options;
mod config;
mod handler_impl;
mod io_utils;
pub(crate) mod policy;
mod python;
#[cfg(test)]
pub(crate) mod test_support;
mod validations;
mod wait_outcomes;
mod worker;

use std::{
    fs::File,
    io::{self, BufWriter, Seek, Write},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use pyo3::prelude::*;

use crate::formatter::{DefaultFormatter, FemtoFormatter};
// The test modules reach `FemtoHandlerTrait::handle` through `use super::*`;
// the production bindings import the trait directly in `python.rs`.
#[cfg(test)]
use crate::handler::FemtoHandlerTrait;
#[cfg(test)]
use crate::level::FemtoLevel;
use crate::shutdown::{ShutdownCompletion, TerminalStatus};
use crate::sync::{
    JoinHandle, Mutex, Receiver, RecvTimeoutError, SendTimeoutError, Sender, bounded,
};
use io_utils::open_log_file;
use worker::{FileCommand, WorkerConfig, spawn_worker};

use super::telemetry::{WaitContext, WaitDiagnostics, WaitOutcome, WaitSite, WaitTimer};

pub(crate) use builder_options::BuilderOptions;
pub use config::{DEFAULT_CHANNEL_CAPACITY, HandlerConfig, OverflowPolicy, TestConfig};
pub(crate) use validations::{
    validate_capacity_nonzero, validate_flush_interval_nonzero, validate_params,
};
pub(crate) use worker::{NoRotation, RotationStrategy};

pub(crate) use wait_outcomes::{EnqueueFailure, FlushOutcome, WorkerShutdown, classify_enqueue};

/// File-based logging handler exposed to Python.
///
/// Spawns a worker thread that writes formatted records to disk using the
/// configuration provided at construction time.
#[pyclass]
pub struct FemtoFileHandler {
    /// Sends records and lifecycle commands to the worker; `None` means the
    /// handler has begun closing and can no longer accept new commands. Shared
    /// so a closer never needs an exclusive borrow while a detached flush call
    /// still holds a shared one.
    tx: Mutex<Option<Sender<FileCommand>>>,
    /// Owns the worker join handle so shutdown waits for buffered writes and
    /// rotation work to finish.
    handle: Mutex<Option<JoinHandle<()>>>,
    /// Broadcast completion signalled after the worker's terminal flush, so
    /// every closer waits for the same shutdown even when another closer has
    /// already taken the join handle.
    completion: Arc<ShutdownCompletion>,
    /// Verdict the worker records as it leaves, read by closers that no
    /// longer own the join handle.
    terminal: Arc<TerminalStatus>,
    /// Selects the producer-side queue behaviour when delivery cannot proceed.
    overflow_policy: OverflowPolicy,
    /// Bounded counters and duration totals for the Python-facing waits.
    diagnostics: Arc<WaitDiagnostics>,
}

impl FemtoFileHandler {
    /// Create a handler writing to `path` with default settings.
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::with_capacity(path, DefaultFormatter, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a handler with a custom queue `capacity` and default drop policy.
    ///
    /// The handler flushes the file after every record (`flush_interval = 1`).
    pub fn with_capacity<P, F>(path: P, formatter: F, capacity: usize) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        let cfg = HandlerConfig {
            capacity,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        };
        Self::with_capacity_flush_policy(path, formatter, cfg)
    }

    /// Create a handler using an explicit [`HandlerConfig`].
    ///
    /// This allows callers to override the queue capacity (> 0), flush interval
    /// (> 0), and overflow policy in a single place.
    pub fn with_capacity_flush_policy<P, F>(
        path: P,
        formatter: F,
        config: HandlerConfig,
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        F: FemtoFormatter + Send + 'static,
    {
        validate_capacity_nonzero(config.capacity)
            .map_err(|msg| io::Error::new(io::ErrorKind::InvalidInput, msg))?;
        validate_flush_interval_nonzero(config.flush_interval)
            .map_err(|msg| io::Error::new(io::ErrorKind::InvalidInput, msg))?;
        let file = open_log_file(path)?;
        Ok(Self::from_file(file, formatter, config))
    }

    /// Starts a worker that owns the buffered file, formatter, and validated
    /// configuration, then returns the handler connected to that worker.
    fn from_file<F>(file: File, formatter: F, config: HandlerConfig) -> Self
    where
        F: FemtoFormatter + Send + 'static,
    {
        // Use a buffered writer so flush policies control when records are
        // persisted to disk. Without buffering each write reaches the OS
        // immediately, causing premature flushes and defeating the configured
        // `flush_interval`.
        let writer = BufWriter::new(file);
        Self::build_from_worker(
            writer,
            formatter,
            config,
            BuilderOptions::<BufWriter<File>>::default(),
        )
    }

    /// Flush the worker's buffer, waiting up to one second. Returns `true` only when the flush
    /// is confirmed in time; `false` if the handler is closed, unreachable or the flush fails.
    pub fn flush(&self) -> bool {
        matches!(self.flush_outcome(), FlushOutcome::Acknowledged)
    }

    /// Flush and report why the attempt finished.
    ///
    /// The Python binding records this outcome so an operator can tell a
    /// timed-out flush from a flush of an already-closed handler, which the
    /// public `bool` cannot express.
    pub(crate) fn flush_outcome(&self) -> FlushOutcome {
        // Bind first: a guard in the `match` scrutinee lives through the send.
        let tx = self.tx.lock().clone();
        match tx {
            Some(tx) => self.perform_flush(&tx),
            None => FlushOutcome::Closed,
        }
    }

    /// Waits for the worker's flush acknowledgement without taking ownership of
    /// the worker thread.
    ///
    /// The enqueue and the acknowledgement share one deadline but are reported
    /// separately: a request that never reached the worker and a request the
    /// worker never answered point at different faults.
    fn perform_flush(&self, tx: &Sender<FileCommand>) -> FlushOutcome {
        let deadline = Instant::now() + Duration::from_secs(1);
        let (ack_tx, ack_rx) = bounded(1);
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tx.send_timeout(FileCommand::Flush(ack_tx), remaining) {
            Ok(()) => {}
            Err(SendTimeoutError::Disconnected(_)) => return FlushOutcome::Disconnected,
            Err(SendTimeoutError::Timeout(_)) => return FlushOutcome::EnqueueTimedOut,
        }
        self.wait_for_flush_completion(&ack_rx, deadline)
    }

    /// Classifies the worker's answer to a flush request.
    fn wait_for_flush_completion(
        &self,
        ack_rx: &Receiver<io::Result<()>>,
        deadline: Instant,
    ) -> FlushOutcome {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match ack_rx.recv_timeout(remaining) {
            Ok(Ok(())) => FlushOutcome::Acknowledged,
            Ok(Err(_)) => FlushOutcome::Failed,
            Err(RecvTimeoutError::Timeout) => FlushOutcome::AckTimedOut,
            // The worker dropped the acknowledgement sender without answering,
            // which means it died between accepting the request and replying.
            Err(RecvTimeoutError::Disconnected) => FlushOutcome::Disconnected,
        }
    }

    /// Return the bounded overflow-policy bucket for telemetry.
    pub(crate) fn policy_bucket(&self) -> &'static str {
        match self.overflow_policy {
            OverflowPolicy::Drop => "drop",
            OverflowPolicy::Block => "block",
            OverflowPolicy::Timeout(_) => "timeout",
        }
    }

    /// Return the shared wait diagnostics for this handler.
    pub(crate) fn diagnostics(&self) -> &Arc<WaitDiagnostics> {
        &self.diagnostics
    }

    /// Time one Python-facing wait and record its outcome.
    ///
    /// The site names the Python class the caller reached, because the
    /// rotating wrappers delegate to this core handler and the operator needs
    /// to tell them apart. The policy bucket comes from this handler, not the
    /// caller, so the label always matches the queue behaviour in force.
    pub(crate) fn record_wait(&self, timer: WaitTimer, site: WaitSite, outcome: WaitOutcome) {
        timer.finish(
            self.diagnostics(),
            WaitContext {
                operation: site.operation,
                handler_kind: site.handler_kind,
                policy_bucket: self.policy_bucket(),
            },
            outcome,
        );
    }

    /// Close the handler and wait for the worker to stop.
    ///
    /// This method is idempotent. Calling it multiple times is safe; only the
    /// first call starts shutdown work, and every call waits on the same
    /// bounded completion signal. A wait that reaches its one-second limit
    /// logs a warning and returns without joining the worker, so a timed-out
    /// call does not confirm that queued records have been written.
    ///
    /// Threading
    /// ---------
    /// The method takes `&self` and may be called concurrently. The first call
    /// takes the worker state; every call waits for the same completion
    /// broadcast, so concurrent closers share one shutdown rather than
    /// competing for the join handle.
    pub fn close(&self) {
        self.take_shutdown().wait();
    }

    /// Marks the handler closed and transfers worker state to the shutdown
    /// operation so it can wait without borrowing the Python object.
    pub(super) fn take_shutdown(&self) -> WorkerShutdown {
        // Each lock is released before `wait`: holding it would block another
        // closer for the whole shutdown, and holding both would invite a
        // lock-order inversion with the worker's own teardown.
        let tx = self.tx.lock().take();
        drop(tx);
        let handle = self.handle.lock().take();
        WorkerShutdown::new(
            handle,
            Arc::clone(&self.completion),
            Arc::clone(&self.terminal),
        )
    }

    /// Builds a handler from an existing worker writer and publishes its
    /// channels only after all configuration has been validated.
    pub(crate) fn build_from_worker<W, F, R>(
        writer: W,
        formatter: F,
        config: HandlerConfig,
        options: BuilderOptions<W, R>,
    ) -> Self
    where
        W: Write + Seek + Send + 'static,
        F: FemtoFormatter + Send + 'static,
        R: RotationStrategy<W> + Send + 'static,
    {
        let BuilderOptions {
            rotation,
            start_barrier,
            ..
        } = options;
        let mut worker_cfg = WorkerConfig::from(&config);
        worker_cfg.start_barrier = start_barrier;
        let overflow_policy = config.overflow_policy;
        let (tx, completion, terminal, handle) =
            spawn_worker(writer, formatter, worker_cfg, rotation);
        Self {
            tx: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
            completion,
            terminal,
            overflow_policy,
            diagnostics: Arc::new(WaitDiagnostics::new()),
        }
    }

    /// Build a handler over the writer in `config` instead of opening a file. For tests: the
    /// settings are used without the validation that file construction applies.
    pub fn with_writer_for_test<W, F>(config: TestConfig<W, F>) -> Self
    where
        W: Write + Seek + Send + 'static,
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
        let handler_config = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy,
        };
        let options = BuilderOptions::<W>::new(NoRotation, start_barrier);
        Self::build_from_worker(writer, formatter, handler_config, options)
    }
}

#[cfg(test)]
mod flush_ack_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod outcome_paths_tests;
#[cfg(test)]
mod scripted_writer;
#[cfg(test)]
mod shutdown_paths_tests;
#[cfg(all(test, feature = "tracing-compat"))]
mod telemetry_event_tests;
#[cfg(test)]
mod telemetry_tests;
#[cfg(test)]
mod tests;
