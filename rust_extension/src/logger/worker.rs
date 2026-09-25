//! Worker-thread lifecycle management for [`FemtoLogger`].
//!
//! These helpers own queue draining, shutdown coordination, and construction of
//! the background logging worker.

use log::warn;
#[cfg(not(loom))]
use pyo3::Python;

use crate::filters::FemtoFilter;
use crate::formatter::{DefaultFormatter, SharedFormatter};
use crate::handler::FemtoHandlerTrait;
use crate::level::FemtoLevel;
use crate::rate_limited_warner::RateLimitedWarner;
use crate::sync::{
    Either, JoinHandle, Mutex, Receiver, RwLock, TryRecvError, bounded, recv_either, spawn,
};

use super::{DEFAULT_CHANNEL_CAPACITY, FemtoLogger, QueuedRecord};

impl FemtoLogger {
    /// Create a logger with an explicit parent name.
    pub fn with_parent(name: String, parent: Option<String>) -> Self {
        let formatter = SharedFormatter::new(DefaultFormatter);
        let handlers: std::sync::Arc<RwLock<Vec<std::sync::Arc<dyn FemtoHandlerTrait>>>> =
            std::sync::Arc::new(RwLock::new(Vec::new()));
        let filters: std::sync::Arc<RwLock<Vec<std::sync::Arc<dyn FemtoFilter>>>> =
            std::sync::Arc::new(RwLock::new(Vec::new()));

        let (tx, rx) = bounded::<QueuedRecord>(DEFAULT_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let handle = spawn(move || {
            Self::worker_thread_loop(rx, shutdown_rx);
        });

        Self {
            name,
            parent,
            formatter,
            level: std::sync::atomic::AtomicU8::new(u8::from(FemtoLevel::Info)),
            propagate: std::sync::atomic::AtomicBool::new(true),
            handlers,
            filters,
            dropped_records: std::sync::atomic::AtomicU64::new(0),
            drop_warner: RateLimitedWarner::default(),
            tx: Some(tx),
            shutdown_tx: Some(shutdown_tx),
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Process a single `FemtoLogRecord` by dispatching it to all handlers.
    pub(crate) fn handle_log_record(job: QueuedRecord) {
        for h in &job.handlers {
            if let Err(err) = h.handle(job.record.clone()) {
                warn!("FemtoLogger: handler reported an error: {err}");
            }
        }
    }

    /// Drain any remaining records once a shutdown signal is received.
    ///
    /// Consumes all messages still available on `rx` and dispatches them
    /// through the provided `handlers`. This ensures no log records are lost
    /// during shutdown.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver holding pending log records.
    pub(crate) fn drain_remaining_records(rx: &Receiver<QueuedRecord>) {
        while let Ok(job) = rx.try_recv() {
            Self::handle_log_record(job);
        }
    }

    /// Finalize the worker thread by draining any remaining queued
    /// records.
    ///
    /// Acts as the shutdown entry point for the worker loop. The
    /// drain step ensures that records already enqueued at the moment
    /// shutdown was signalled are not silently lost.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver holding pending log records.
    pub(crate) fn shutdown_and_drain(rx: &Receiver<QueuedRecord>) {
        Self::drain_remaining_records(rx);
    }

    /// Perform a non-blocking check for a pending shutdown signal.
    ///
    /// This is the Phase 1 check in the two-phase shutdown pattern
    /// used by [`worker_thread_loop`]. It uses `try_recv` rather
    /// than a blocking receive so the worker can detect a shutdown
    /// request that arrived while the previous `recv_either` wait
    /// was busy processing a log record. Without this check, a
    /// continuously saturated record channel could delay shutdown
    /// recognition indefinitely.
    ///
    /// Returns `true` when the shutdown channel carries a message or
    /// has been disconnected.
    ///
    /// # Arguments
    ///
    /// * `shutdown_rx` - Channel receiver carrying the shutdown
    ///   signal.
    pub(crate) fn should_shutdown_now(shutdown_rx: &Receiver<()>) -> bool {
        matches!(
            shutdown_rx.try_recv(),
            Ok(()) | Err(TryRecvError::Disconnected)
        )
    }

    /// Main loop executed by the logger's worker thread.
    ///
    /// Uses a two-phase shutdown pattern to guarantee prompt shutdown
    /// even under sustained high-throughput logging:
    ///
    /// - **Phase 1** ([`should_shutdown_now`]): A non-blocking
    ///   `try_recv` on the shutdown channel, executed at the top of
    ///   every iteration *before* the blocking `recv_either`. This
    ///   provides a deterministic opportunity to observe a shutdown
    ///   signal that arrived while the previous iteration was
    ///   servicing a log record.
    ///
    /// - **Phase 2** (`recv_either`): A blocking wait on both the
    ///   shutdown and record channels. Although `crossbeam`'s
    ///   `select!`, which `recv_either` uses outside Loom, picks at
    ///   random when multiple channels are
    ///   ready, a continuously saturated record channel could still
    ///   cause the shutdown branch to lose repeated coin-flips,
    ///   delaying exit. Phase 1 eliminates this probabilistic delay
    ///   by guaranteeing that every loop iteration checks for
    ///   shutdown deterministically.
    ///
    /// When either phase detects a shutdown signal, all remaining
    /// queued records are drained before the thread exits so that no
    /// log messages are silently lost.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver for incoming log records.
    /// * `shutdown_rx` - Channel receiver carrying the shutdown
    ///   signal, sent by [`FemtoLogger::drop`].
    pub(crate) fn worker_thread_loop(rx: Receiver<QueuedRecord>, shutdown_rx: Receiver<()>) {
        loop {
            if Self::should_shutdown_now(&shutdown_rx) {
                Self::shutdown_and_drain(&rx);
                break;
            }
            match recv_either(&shutdown_rx, &rx) {
                Either::First(_) => {
                    Self::shutdown_and_drain(&rx);
                    break;
                }
                Either::Second(Ok(job)) => Self::handle_log_record(job),
                Either::Second(Err(_)) => break,
            }
        }
    }
}

pub(super) fn log_join_result(handle: JoinHandle<()>) {
    if handle.join().is_err() {
        warn!("FemtoLogger: worker thread panicked");
    }
}

/// Join the logger's worker with the GIL released.
///
/// The worker may be dispatching to a Python handler that needs the GIL, so
/// holding it across the join could deadlock.
#[cfg(not(loom))]
pub(super) fn join_worker(handle: JoinHandle<()>) {
    Python::attach(|py| py.detach(move || log_join_result(handle)));
}

/// Join the logger's worker directly inside a Loom model.
///
/// The interpreter is outside anything Loom schedules, and no Loom model
/// attaches a Python handler, so there is no GIL to release. Attaching would
/// also initialize the interpreter on a Loom coroutine's small stack.
#[cfg(loom)]
pub(super) fn join_worker(handle: JoinHandle<()>) {
    log_join_result(handle);
}
