//! Producer-path filtering and dispatch for [`FemtoLogger`].
//!
//! These helpers keep the hot logging path focused and separate it from the
//! worker-thread lifecycle code.

use std::time::{Duration, Instant};

use log::warn;
#[cfg(not(loom))]
use pyo3::Python;

use crate::filters::FilterContext;
use crate::handler::FemtoHandlerTrait;
use crate::level::FemtoLevel;
use crate::log_context;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::manager;
use crate::sync::bounded;

use super::{FemtoLogger, FlushAckHandler, LOGGER_FLUSH_TIMEOUT_MS, QueuedRecord};

impl FemtoLogger {
    /// Core logging logic shared between Python and Rust APIs.
    ///
    /// Checks level threshold and filters, formats the record, and dispatches
    /// to handlers. Returns `Some(formatted_message)` if the record was logged,
    /// or `None` if it was filtered out.
    pub(super) fn log_record(&self, mut record: FemtoLogRecord) -> Option<String> {
        let threshold = self.level.load(std::sync::atomic::Ordering::Relaxed);
        if u8::from(record.level()) < threshold {
            return None;
        }

        if !self.apply_filters(&mut record) {
            return None;
        }
        let msg = self.formatter.format(&record);
        self.dispatch_to_handlers(record);
        Some(msg)
    }

    /// Log a message at the given level (Rust-only API).
    ///
    /// This method is a simplified version of the PyO3-exposed `log` method
    /// for pure Rust callers. It does not support `exc_info` or `stack_info`.
    pub fn log(&self, level: FemtoLevel, message: &str) -> Option<String> {
        let record = FemtoLogRecord::new(&self.name, level, message);
        self.log_record(record)
    }

    /// Log a message with explicit source location metadata.
    ///
    /// Used by the [`femtolog_info!`] family of macros and the Python
    /// convenience functions to attach caller-captured source information
    /// (filename, line number, module path) to the record before filtering
    /// and dispatch.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use femtologging_rs::{FemtoLevel, FemtoLogger, RecordMetadata};
    ///
    /// let logger = FemtoLogger::new("example".into());
    /// let metadata = RecordMetadata {
    ///     filename: "main.rs".into(),
    ///     line_number: 42,
    ///     module_path: "example".into(),
    ///     ..Default::default()
    /// };
    /// logger.log_with_metadata(FemtoLevel::Info, "hello", metadata);
    /// ```
    pub fn log_with_metadata(
        &self,
        level: FemtoLevel,
        message: &str,
        mut metadata: RecordMetadata,
    ) -> Option<String> {
        if !self.is_enabled_for(level) {
            return None;
        }
        match log_context::merge_context_values(&metadata.key_values) {
            Ok(merged_key_values) => metadata.key_values = merged_key_values,
            Err(err) => {
                eprintln!("FemtoLogger: dropping record due to invalid context payload: {err}");
                return None;
            }
        }
        let record = FemtoLogRecord::with_metadata(&self.name, level, message, metadata);
        self.log_record(record)
    }

    /// Return whether `level` is enabled for this logger.
    pub(crate) fn is_enabled_for(&self, level: FemtoLevel) -> bool {
        u8::from(level) >= self.level.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Dispatch an already-constructed record through this logger.
    ///
    /// The record is filtered against the logger's level and filters before
    /// being enqueued for handler processing.
    #[cfg(any(feature = "log-compat", feature = "tracing-compat"))]
    pub(crate) fn dispatch_record(&self, record: FemtoLogRecord) {
        let mut record = record;
        if !self.is_enabled_for(record.level()) {
            return;
        }
        if !self.apply_filters(&mut record) {
            return;
        }
        self.dispatch_to_handlers(record);
    }

    /// Return the logger's current minimum level.
    ///
    /// This method is thread-safe; the level is stored in an `AtomicU8` and
    /// read with `Ordering::Relaxed`.
    pub fn get_level(&self) -> FemtoLevel {
        self.load_level()
    }

    /// Load the current level from the atomic storage.
    ///
    /// The level is guaranteed to be a valid `FemtoLevel` discriminant because
    /// the only way to set it is through `set_level`, which accepts a typed
    /// `FemtoLevel` value.
    pub(super) fn load_level(&self) -> FemtoLevel {
        match self.level.load(std::sync::atomic::Ordering::Relaxed) {
            0 => FemtoLevel::Trace,
            1 => FemtoLevel::Debug,
            2 => FemtoLevel::Info,
            3 => FemtoLevel::Warn,
            4 => FemtoLevel::Error,
            _ => FemtoLevel::Critical,
        }
    }

    /// Return `true` if every configured filter approves the record.
    ///
    /// Iterates over each filter and returns `false` on the first rejection.
    /// If no filters are configured, the record passes.
    fn apply_filters(&self, record: &mut FemtoLogRecord) -> bool {
        let filters = self.filters.read().clone();
        let mut context = FilterContext::default();
        for filter in filters {
            let decision = filter.decision(record, &mut context);
            if !decision.accepted {
                return false;
            }
            record.metadata_mut().key_values.extend(decision.enrichment);
        }
        true
    }

    fn should_propagate_to_parent(&self) -> bool {
        self.propagate.load(std::sync::atomic::Ordering::SeqCst) && self.parent.is_some()
    }

    fn handle_parent_propagation(&self, record: FemtoLogRecord) {
        let Some(parent_name) = &self.parent else {
            return;
        };
        pyo3::Python::attach(|py| {
            if let Ok(parent) = manager::get_logger(py, parent_name) {
                parent.borrow(py).dispatch_to_handlers(record);
            }
        });
    }

    fn send_to_local_handlers(&self, record: FemtoLogRecord) {
        let Some(tx) = &self.tx else {
            return;
        };
        let handlers = self.handlers.read().clone();
        if tx.try_send(QueuedRecord { record, handlers }).is_ok() {
            return;
        }
        self.dropped_records
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.drop_warner.record_drop();
        self.drop_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records; queue full or shutting down");
        });
    }

    /// Flush every handler attached to this logger.
    ///
    /// The flush is a barrier: a marker record is queued behind everything
    /// already queued, so the marker being acknowledged means the records
    /// ahead of it have been dispatched. One absolute deadline covers the
    /// whole barrier — enqueueing the marker and waiting for the
    /// acknowledgement — because either half stalls on the same full queue.
    /// Charging each half its own timeout let a full queue spend the
    /// advertised budget twice, and a blocking enqueue could overshoot it
    /// without bound.
    ///
    /// Returns `false` when the marker could not be enqueued before the
    /// deadline (the queue stayed full, or the logger is shutting down), when
    /// the worker did not acknowledge it before the deadline (a handler is
    /// stalled), when the worker has already exited, or when any attached
    /// handler's `flush()` reports failure.
    pub(super) fn flush_handlers_blocking(&self) -> bool {
        self.wait_for_worker_idle_ungil() && self.flush_configured_handlers()
    }

    /// Wait for the worker to acknowledge a flush marker.
    ///
    /// Returns `true` immediately when this logger has no worker thread:
    /// with no queue there is nothing to drain.
    ///
    /// Call [`FemtoLogger::wait_for_worker_idle_ungil`] rather than this
    /// method directly: the waits here block, and the worker may need the GIL
    /// to make progress. See that method for why.
    fn wait_for_worker_idle(&self) -> bool {
        let Some(tx) = &self.tx else {
            return true;
        };
        let deadline = Instant::now() + Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS);
        let (ack_tx, ack_rx) = bounded(1);
        let ack_handler: std::sync::Arc<dyn FemtoHandlerTrait> =
            std::sync::Arc::new(FlushAckHandler::new(ack_tx));
        let record = FemtoLogRecord::new("__femtologging__", FemtoLevel::Info, "__flush__");
        let marker = QueuedRecord {
            record,
            handlers: vec![ack_handler],
        };
        // Both failures reach the caller as the same verdict — the worker
        // never confirmed the barrier. `Timeout` is a saturated queue or a
        // stalled worker; `Disconnected` is a worker that has already exited.
        if tx.send_timeout(marker, remaining(deadline)).is_err() {
            return false;
        }
        ack_rx.recv_timeout(remaining(deadline)).is_ok()
    }

    fn flush_configured_handlers(&self) -> bool {
        self.handlers.read().iter().all(|handler| handler.flush())
    }

    /// Wait for the worker to acknowledge a flush marker, without the GIL.
    ///
    /// See [`FemtoLogger::wait_for_worker_idle`] for why the wait must not
    /// hold the GIL. `Python::attach` is a no-op when this thread already
    /// holds the interpreter, so the release happens exactly where the
    /// blocking wait does and the caller's callbacks are unaffected.
    #[cfg(not(loom))]
    pub(super) fn wait_for_worker_idle_ungil(&self) -> bool {
        Python::attach(|py| py.detach(|| self.wait_for_worker_idle()))
    }

    /// Wait for the worker directly inside a Loom model.
    ///
    /// No Loom model attaches a Python handler, so there is no GIL to release;
    /// see [`super::worker::join_worker`] for the same distinction.
    #[cfg(loom)]
    pub(super) fn wait_for_worker_idle_ungil(&self) -> bool {
        self.wait_for_worker_idle()
    }

    /// Dispatch a record to the logger's handlers via the background queue.
    ///
    /// The record is sent to the worker thread if a channel is configured.
    /// When the queue is full or the logger is shutting down, the record is
    /// dropped; a drop counter increments and a rate-limited warning is
    /// emitted.
    pub(super) fn dispatch_to_handlers(&self, record: FemtoLogRecord) {
        let parent_record = self.should_propagate_to_parent().then(|| record.clone());
        self.send_to_local_handlers(record);
        if let Some(pr) = parent_record {
            self.handle_parent_propagation(pr);
        }
    }
}

/// Time left before `deadline`, or zero when it has already passed.
///
/// A single deadline covers the whole flush barrier, so each wait in it is
/// given what remains rather than the full budget; a deadline in the past
/// yields zero, which the timed channel operations treat as a non-blocking
/// attempt rather than a panic.
///
/// # Examples
///
/// ```ignore
/// let deadline = Instant::now() + Duration::from_millis(50);
/// assert!(remaining(deadline) <= Duration::from_millis(50));
/// assert_eq!(remaining(Instant::now()), Duration::ZERO);
/// ```
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}
