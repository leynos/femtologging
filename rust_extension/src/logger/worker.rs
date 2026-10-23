//! Construction of [`FemtoLogger`] and the dispatch shard it draws from.
//!
//! A logger no longer owns a worker thread. Construction resolves the
//! process-wide dispatch pool and takes a shard from it, and that is the one
//! step which can fail: a process at its thread limit cannot start the pool,
//! and the refusal is reported to the caller rather than panicking inside
//! `thread::spawn`.
//!
//! Under `--cfg loom` there is no shared pool. `loom::model` runs its closure
//! repeatedly in one process, so a pool started in one iteration would still be
//! running, blocked on its channels, in the next. A model therefore owns one
//! shard, and its logger's drop is what ends that shard's worker. The models in
//! `tests/heavy/` assert message routing and handler invocation counts, neither
//! of which the size of the pool changes.

use crate::formatter::{DefaultFormatter, SharedFormatter};
use crate::level::FemtoLevel;
use crate::rate_limited_warner::RateLimitedWarner;
use crate::sync::RwLock;
#[cfg(feature = "python")]
use super::context_snapshot::{ContextSnapshotProvider, ContextVarsSnapshotProvider};

use super::FemtoLogger;
use super::HandlerAttachment;
use super::Shard;
#[cfg(not(loom))]
use super::dispatcher::Dispatcher;
use super::dispatcher::DispatcherError;
#[cfg(loom)]
use super::dispatcher::LOOM_RECORDS_PER_LOGGER;

/// How a logger refers to the shard its records are dispatched on.
///
/// A process-wide pool outlives every logger, so a logger borrows its shard;
/// nothing has to be kept alive in the other direction. A Loom model has no
/// pool: it builds a shard for the run and must join that shard's worker before
/// the run ends, so the logger owns the shard and its drop is what ends it.
#[cfg(not(loom))]
type ShardHandle = &'static Shard;
#[cfg(loom)]
type ShardHandle = std::sync::Arc<Shard>;

impl FemtoLogger {
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
    /// Loom's thread builder cannot fail, so this arm cannot either. It keeps
    /// the shared `Result` signature only so both arms present one call site;
    /// there is no `DispatcherError` to build here, because a Loom model has no
    /// pool whose partial start the error would have to describe.
    #[cfg(loom)]
    pub fn with_parent(name: String, parent: Option<String>) -> Result<Self, DispatcherError> {
        let shard = Shard::start(0, LOOM_RECORDS_PER_LOGGER)
            .expect("Loom's thread builder cannot fail to start a shard worker");
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
        // was given, so it is created from that shard's depth.
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
            shard,
            budget,
        }
    }
}
