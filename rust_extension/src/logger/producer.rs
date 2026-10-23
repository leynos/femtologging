//! Producer-path filtering and dispatch for [`FemtoLogger`].
//!
//! These helpers keep the hot logging path focused and separate it from the
//! dispatch pool. Nothing here waits: a record is either handed to the shard or
//! counted as dropped, so a slow handler slows the loggers sharing its shard
//! rather than the code doing the logging.

use log::warn;

use crate::filters::FilterContext;
use crate::level::FemtoLevel;
use crate::log_context;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::manager;

use super::{FemtoLogger, QueuedRecord};

/// Classify handlers without retaining their identities in a tracing event.
#[cfg(feature = "tracing-compat")]
fn handler_kind(handlers: &[std::sync::Arc<dyn crate::handler::FemtoHandlerTrait>]) -> &'static str {
    let has_python = handlers.iter().any(|handler| handler.is_python_backed());
    let has_native = handlers.iter().any(|handler| !handler.is_python_backed());
    match (has_python, has_native) {
        (true, true) => "python_and_native",
        (true, false) => "python",
        (false, true) => "native",
        (false, false) => "none",
    }
}

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
                self.context_dropped_records
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                eprintln!("FemtoLogger: dropping record due to invalid context payload: {err}");
                return None;
            }
        }
        let record = FemtoLogRecord::with_metadata(&self.name, level, message, metadata);
        self.log_record(record)
    }

    /// Log a record whose context was captured by the Python caller.
    ///
    /// Python task-local context is already present in `metadata`; Rust
    /// thread-local context is deliberately not merged a second time.
    #[cfg(feature = "python")]
    pub(crate) fn log_with_explicit_metadata(
        &self,
        level: FemtoLevel,
        message: &str,
        metadata: RecordMetadata,
    ) -> Option<String> {
        if !self.is_enabled_for(level) {
            return None;
        }
        let record = FemtoLogRecord::with_metadata(&self.name, level, message, metadata);
        self.log_record(record)
    }

    /// Return whether `level` is enabled for this logger.
    pub(crate) fn is_enabled_for(&self, level: FemtoLevel) -> bool {
        u8::from(level) >= self.level.load(std::sync::atomic::Ordering::Relaxed)
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
        // Captured here, on the logging thread, so the record is dispatched to
        // the handlers that were installed when it was logged.
        let handlers = self.handlers.read().clone();
        #[cfg(feature = "tracing-compat")]
        let handler_kind = handler_kind(&handlers);
        #[cfg(feature = "python")]
        let context = match self.capture_context_for_dispatch(
            &handlers,
            #[cfg(feature = "tracing-compat")]
            handler_kind,
        ) {
            Ok(context) => context,
            Err(error) => {
                self.record_context_capture_failure(error);
                return;
            }
        };
        #[cfg(feature = "tracing-compat")]
        let dispatch_started = std::time::Instant::now();
        let queued = Box::new(QueuedRecord {
            record,
            handlers,
            #[cfg(feature = "python")]
            context,
        });
        if self.shard.admit(queued).is_ok() {
            #[cfg(feature = "tracing-compat")]
            tracing::trace!(
                target: "femtologging::internal",
                operation = "producer_queue_dispatch",
                handler_kind,
                dispatch_outcome = "queued",
                elapsed_us = dispatch_started.elapsed().as_micros(),
            );
            return;
        }
        #[cfg(feature = "tracing-compat")]
        tracing::trace!(
            target: "femtologging::internal",
            operation = "producer_queue_dispatch",
            handler_kind,
            dispatch_outcome = "rejected",
            elapsed_us = dispatch_started.elapsed().as_micros(),
        );
        self.dropped_records
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.drop_warner.record_drop();
        self.drop_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records; dispatch queue full");
        });
    }

    /// Capture Python context for the exact handler snapshot being queued.
    #[cfg(feature = "python")]
    fn capture_context_for_dispatch(
        &self,
        handlers: &[std::sync::Arc<dyn crate::handler::FemtoHandlerTrait>],
        #[cfg(feature = "tracing-compat")] handler_kind: &'static str,
    ) -> pyo3::PyResult<Option<pyo3::Py<pyo3::PyAny>>> {
        #[cfg(feature = "tracing-compat")]
        let capture_started = std::time::Instant::now();
        let context = self.capture_python_context(handlers);
        #[cfg(feature = "tracing-compat")]
        {
            let capture_outcome = match &context {
                Ok(Some(_)) => "captured",
                Ok(None) => "not_required",
                Err(_) => "failed",
            };
            tracing::trace!(
                target: "femtologging::internal",
                operation = "producer_context_capture",
                handler_kind,
                capture_outcome,
                elapsed_us = capture_started.elapsed().as_micros(),
            );
        }
        context
    }

    /// Count a failed context snapshot separately from queue-capacity drops.
    #[cfg(feature = "python")]
    fn record_context_capture_failure(&self, error: pyo3::PyErr) {
        self.context_capture_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.context_capture_warner.record_drop();
        self.context_capture_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records after context capture failed");
            pyo3::Python::attach(|py| error.print(py));
        });
    }

    /// Capture the current Python context only when a queued handler needs it.
    #[cfg(feature = "python")]
    pub(super) fn capture_python_context(
        &self,
        handlers: &[std::sync::Arc<dyn crate::handler::FemtoHandlerTrait>],
    ) -> pyo3::PyResult<Option<pyo3::Py<pyo3::PyAny>>> {
        if !handlers.iter().any(|handler| handler.is_python_backed()) {
            return Ok(None);
        }
        self.context_snapshot_provider.capture().map(Some)
    }

    pub(super) fn flush_handlers_blocking(&self) -> bool {
        self.wait_for_dispatch_to_drain() && self.flush_configured_handlers()
    }

    /// Wait for the logger's shard to finish the records queued ahead.
    ///
    /// The barrier travels the shard's own queue, so it cannot overtake a
    /// record this logger logged first, and a shard that is gone or too slow
    /// answers `false` rather than waiting indefinitely.
    fn wait_for_dispatch_to_drain(&self) -> bool {
        self.shard.wait_for_drain()
    }

    fn flush_configured_handlers(&self) -> bool {
        self.handlers.read().iter().all(|handler| handler.flush())
    }

    /// Dispatch a record to the logger's handlers through its dispatch shard.
    ///
    /// When the shard is full the record is dropped; a drop counter increments
    /// and a rate-limited warning is emitted. Nothing here blocks.
    pub(super) fn dispatch_to_handlers(&self, record: FemtoLogRecord) {
        let parent_record = self.should_propagate_to_parent().then(|| record.clone());
        self.send_to_local_handlers(record);
        if let Some(pr) = parent_record {
            self.handle_parent_propagation(pr);
        }
    }
}
