//! Construction of [`FemtoLogger`] and the dispatch shard it draws from.
//!
//! A logger no longer owns a worker thread. Construction resolves the
//! process-wide dispatch pool and takes a shard from it. A logger inherited
//! across `fork` rebinds its shard on first dispatch because its parent's worker
//! threads did not survive; either pool start can fail at the process thread
//! limit and is reported rather than panicking inside `thread::spawn`.
//!
//! Under `--cfg loom` there is no shared pool. `loom::model` runs its closure
//! repeatedly in one process, so a pool started in one iteration would still be
//! running, blocked on its channels, in the next. A model therefore owns one
//! shard, and its logger's drop is what ends that shard's worker. The models in
//! `tests/heavy/` assert message routing and handler invocation counts, neither
//! of which the size of the pool changes.

#[cfg(feature = "python")]
use super::context_snapshot::{ContextSnapshotProvider, ContextVarsSnapshotProvider};
use crate::formatter::{DefaultFormatter, SharedFormatter};
use crate::level::FemtoLevel;
use crate::rate_limited_warner::RateLimitedWarner;
use crate::sync::RwLock;

use super::FemtoLogger;
use super::HandlerAttachment;
#[cfg(not(loom))]
use super::LoggerBudget;
use super::Shard;
#[cfg(not(loom))]
use super::dispatcher::Dispatcher;
use super::dispatcher::DispatcherError;
#[cfg(loom)]
use super::dispatcher::LOOM_RECORDS_PER_LOGGER;

/// How a logger refers to the shard its records are dispatched on.
///
/// A process-wide pool outlives every logger, so each process-specific dispatch
/// state borrows its shard; nothing has to be kept alive in the other direction.
/// A Loom model has no pool: it builds a shard for the run and must join that
/// shard's worker before the run ends, so the logger owns the shard and its
/// drop is what ends it.
#[cfg(not(loom))]
type ShardHandle = &'static Shard;
#[cfg(loom)]
type ShardHandle = std::sync::Arc<Shard>;

/// The shard and admission budget assigned to one logger in one process.
///
/// A logger may survive a `fork`, but its parent's worker threads do not. The
/// process ID lets the logger replace this state before its next dispatch.
#[cfg(not(loom))]
pub(super) struct DispatchState {
    /// Process whose dispatcher owns the shard pointer.
    process_id: std::sync::atomic::AtomicU32,
    /// PID reserving the one-time rebind after a fork, or zero when free.
    starting_process: std::sync::atomic::AtomicU32,
    /// Shard assigned to this logger in `process_id`.
    shard: std::sync::atomic::AtomicPtr<Shard>,
    /// This logger's admission budget on `shard`.
    budget: std::sync::Arc<LoggerBudget>,
}

/// Release a logger's process-local rebind reservation after success or error.
#[cfg(not(loom))]
struct DispatchRebindGuard<'a> {
    /// Atomic reservation owned by the logger.
    starting_process: &'a std::sync::atomic::AtomicU32,
    /// PID that claimed the reservation.
    process_id: u32,
}

#[cfg(not(loom))]
impl Drop for DispatchRebindGuard<'_> {
    fn drop(&mut self) {
        let _ = self.starting_process.compare_exchange(
            self.process_id,
            0,
            std::sync::atomic::Ordering::Release,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

#[cfg(not(loom))]
impl DispatchState {
    /// Build this logger's process-local shard and in-flight budget.
    fn new(shard: &'static Shard) -> Self {
        Self {
            process_id: std::sync::atomic::AtomicU32::new(std::process::id()),
            starting_process: std::sync::atomic::AtomicU32::new(0),
            shard: std::sync::atomic::AtomicPtr::new(shard as *const Shard as *mut Shard),
            budget: shard.budget(),
        }
    }

    /// Return the shard and budget after the process ID has been checked.
    fn handles(&self) -> (&Shard, &std::sync::Arc<LoggerBudget>) {
        let shard = self.shard.load(std::sync::atomic::Ordering::Acquire);
        // SAFETY: construction and rebind publish a shard from the process-wide
        // pool, which outlives logger state and is never freed after a fork.
        (unsafe { &*shard }, &self.budget)
    }
}

impl FemtoLogger {
    /// Return this logger's dispatch state for the current process.
    ///
    /// A child inherits logger objects but not its parent's worker threads.
    /// Replacing the state on first use routes even those inherited loggers to
    /// a shard in the child's newly started pool.
    ///
    /// # Errors
    ///
    /// Returns [`DispatcherError`] if the child cannot start its replacement
    /// pool. The failed start is not cached, so another call can retry.
    #[cfg(not(loom))]
    pub(super) fn dispatch_state(
        &self,
    ) -> Result<(&Shard, &std::sync::Arc<LoggerBudget>), DispatcherError> {
        let process_id = std::process::id();
        loop {
            if self
                .dispatch
                .process_id
                .load(std::sync::atomic::Ordering::Acquire)
                == process_id
            {
                return Ok(self.dispatch.handles());
            }

            match self.dispatch.starting_process.compare_exchange(
                0,
                process_id,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            ) {
                Ok(_) => {
                    let _guard = DispatchRebindGuard {
                        starting_process: &self.dispatch.starting_process,
                        process_id,
                    };
                    if self
                        .dispatch
                        .process_id
                        .load(std::sync::atomic::Ordering::Acquire)
                        == process_id
                    {
                        return Ok(self.dispatch.handles());
                    }

                    let dispatcher = Dispatcher::shared()?;
                    let shard = dispatcher.assign_shard();
                    self.dispatch.budget.reset_after_fork();
                    self.dispatch.shard.store(
                        shard as *const Shard as *mut Shard,
                        std::sync::atomic::Ordering::Release,
                    );
                    self.dispatch
                        .process_id
                        .store(process_id, std::sync::atomic::Ordering::Release);
                    return Ok((shard, &self.dispatch.budget));
                }
                Err(owner) if owner == process_id => std::thread::yield_now(),
                Err(owner) => {
                    // A different PID can only be the parent's reservation
                    // copied by fork. The child may clear it and retry.
                    let _ = self.dispatch.starting_process.compare_exchange(
                        owner,
                        0,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Acquire,
                    );
                }
            }
        }
    }

    /// Drain work this logger sent to its current dispatch state.
    ///
    /// State inherited from a parent process is left untouched: its shard may
    /// still be borrowed by another inherited logger in the child.
    #[cfg(not(loom))]
    pub(super) fn finish_dispatch_state(&mut self) {
        if self
            .dispatch
            .process_id
            .load(std::sync::atomic::Ordering::Acquire)
            != std::process::id()
        {
            return;
        }
        let (shard, budget) = self.dispatch.handles();
        if budget.in_flight() > 0 {
            // The wait is taken while detached from the Python interpreter,
            // because a handler on the shard may be a Python object that needs
            // the GIL. When no interpreter is attached, no Python handler can
            // be mid-call either, so wait directly.
            if pyo3::Python::try_attach(|py| py.detach(|| shard.wait_for_drain())).is_none() {
                let _ = shard.wait_for_drain();
            }
        }
    }

    /// Create a logger with an explicit parent name.
    ///
    /// # Errors
    ///
    /// Returns [`DispatcherError`] when the process-wide dispatch pool is not
    /// running and the system refuses to start the threads it needs.
    #[cfg(not(loom))]
    pub fn with_parent(name: String, parent: Option<String>) -> Result<Self, DispatcherError> {
        let dispatcher = Dispatcher::shared()?;
        Ok(Self::assembled(
            name,
            parent,
            dispatcher.assign_shard(),
            #[cfg(feature = "python")]
            std::sync::Arc::new(ContextVarsSnapshotProvider),
        ))
    }

    /// Create a logger that owns the only shard in its Loom model.
    ///
    /// Loom's thread builder cannot fail in practice, but the signature is
    /// `Result` for the same reason the pool's is: a refusal to start a worker
    /// is a resource answer the caller reports, not a panic. Propagating it
    /// keeps the two arms' error handling identical, so neither has a reason
    /// for `.expect()` that the repository forbids outside tests.
    ///
    /// # Errors
    ///
    /// Returns [`DispatcherError`] when the shard's worker cannot be started.
    #[cfg(loom)]
    pub fn with_parent(name: String, parent: Option<String>) -> Result<Self, DispatcherError> {
        let shard = Shard::start(0, LOOM_RECORDS_PER_LOGGER)
            .map_err(|source| DispatcherError::new(0, source))?;
        Ok(Self::assembled(
            name,
            parent,
            std::sync::Arc::new(shard),
            #[cfg(feature = "python")]
            std::sync::Arc::new(ContextVarsSnapshotProvider),
        ))
    }

    /// Create a logger with a test-controlled context snapshot provider.
    #[cfg(all(feature = "python", test, not(loom)))]
    pub(super) fn with_context_snapshot_provider(
        name: String,
        parent: Option<String>,
        context_snapshot_provider: std::sync::Arc<dyn ContextSnapshotProvider>,
    ) -> Self {
        let dispatcher = Dispatcher::shared().expect("test dispatcher should start");
        Self::assembled(
            name,
            parent,
            dispatcher.assign_shard(),
            context_snapshot_provider,
        )
    }

    /// Assemble a logger on a shard a test has chosen.
    ///
    /// Tests that assert what one logger's flush does to a shard it *shares*
    /// have to place the logger on that shard; the process-wide pool assigns
    /// round robin and gives no control over which logger lands where. Outside
    /// Loom only, like the pool it borrows from.
    #[cfg(all(test, not(loom)))]
    pub(crate) fn assembled_for_test(name: String, shard: ShardHandle) -> Self {
        Self::assembled(
            name,
            None,
            shard,
            #[cfg(feature = "python")]
            std::sync::Arc::new(ContextVarsSnapshotProvider),
        )
    }

    /// Assemble a logger around an already-chosen shard.
    ///
    /// The shard is the logger's whole relationship with dispatch: every record
    /// it logs goes to that one shard, and a shard runs its queue in order, so
    /// one logger's records are handled in the order they were logged.
    fn assembled(
        name: String,
        parent: Option<String>,
        shard: ShardHandle,
        #[cfg(feature = "python")] context_snapshot_provider: std::sync::Arc<
            dyn ContextSnapshotProvider,
        >,
    ) -> Self {
        // The budget belongs to this logger and is enforced by the shard it
        // was given, so it is derived from that shard. The shard sizes the
        // budget below its depth so one logger can never take every permit.
        #[cfg(not(loom))]
        let dispatch = DispatchState::new(shard);
        #[cfg(loom)]
        let budget = shard.budget();
        Self {
            name,
            parent,
            formatter: SharedFormatter::new(DefaultFormatter),
            level: std::sync::atomic::AtomicU8::new(u8::from(FemtoLevel::Info)),
            propagate: std::sync::atomic::AtomicBool::new(true),
            handlers: std::sync::Arc::new(RwLock::new(Vec::<HandlerAttachment>::new())),
            filters: std::sync::Arc::new(RwLock::new(Vec::new())),
            dropped_records: std::sync::atomic::AtomicU64::new(0),
            context_dropped_records: std::sync::atomic::AtomicU64::new(0),
            drop_warner: RateLimitedWarner::default(),
            #[cfg(feature = "python")]
            context_snapshot_provider,
            #[cfg(feature = "python")]
            context_capture_failures: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "python")]
            context_capture_warner: RateLimitedWarner::default(),
            #[cfg(not(loom))]
            dispatch,
            #[cfg(loom)]
            shard,
            #[cfg(loom)]
            budget,
        }
    }
}
