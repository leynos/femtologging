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
use crate::shutdown::ShutdownCompletion;
use crate::sync::{JoinHandle, Mutex, Receiver, Sender, bounded};
use io_utils::open_log_file;
use worker::{FileCommand, WorkerConfig, spawn_worker};

pub(crate) use builder_options::BuilderOptions;
pub use config::{DEFAULT_CHANNEL_CAPACITY, HandlerConfig, OverflowPolicy, TestConfig};
pub(crate) use validations::{
    validate_capacity_nonzero, validate_flush_interval_nonzero, validate_params,
};
pub(crate) use worker::{NoRotation, RotationStrategy};

/// Owns the worker state needed to finish a handler shutdown after its Python
/// object borrow has been released.
pub(super) struct WorkerShutdown {
    /// Worker thread handle retained for joining after completion.
    handle: Option<JoinHandle<()>>,
    /// Broadcast completion every closer waits on, so a closer that does not
    /// own the join handle still observes worker shutdown.
    completion: Arc<ShutdownCompletion>,
}

impl WorkerShutdown {
    /// Wait for worker completion and join it when it reports a clean exit.
    ///
    /// Every caller waits on the shared completion, including one that finds
    /// no join handle left: another closer may still be joining a worker that
    /// has not finished writing its queued records.
    pub(super) fn wait(self) {
        if !self.completion.wait(Duration::from_secs(1)) {
            log::warn!("FemtoFileHandler: worker thread did not shut down within 1s");
            return;
        }
        if let Some(handle) = self.handle
            && handle.join().is_err()
        {
            log::warn!("FemtoFileHandler: worker thread panicked");
        }
    }
}

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
    /// Selects the producer-side queue behaviour when delivery cannot proceed.
    overflow_policy: OverflowPolicy,
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
        // Bind first: a guard in the `match` scrutinee lives through the send.
        let tx = self.tx.lock().clone();
        match tx {
            Some(tx) => self.perform_flush(&tx),
            None => false,
        }
    }

    /// Waits for the worker's flush acknowledgement without taking ownership of
    /// the worker thread.
    fn perform_flush(&self, tx: &Sender<FileCommand>) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        let (ack_tx, ack_rx) = bounded(1);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if tx
            .send_timeout(FileCommand::Flush(ack_tx), remaining)
            .is_err()
        {
            return false;
        }
        self.wait_for_flush_completion(&ack_rx, deadline)
    }

    /// Returns true only when the worker acknowledges the flush successfully
    /// before the caller's deadline.
    fn wait_for_flush_completion(
        &self,
        ack_rx: &Receiver<io::Result<()>>,
        deadline: Instant,
    ) -> bool {
        let remaining = deadline.saturating_duration_since(Instant::now());
        matches!(ack_rx.recv_timeout(remaining), Ok(Ok(())))
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
        WorkerShutdown {
            handle,
            completion: Arc::clone(&self.completion),
        }
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
        let (tx, completion, handle) = spawn_worker(writer, formatter, worker_cfg, rotation);
        Self {
            tx: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
            completion,
            overflow_policy,
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
mod tests;
