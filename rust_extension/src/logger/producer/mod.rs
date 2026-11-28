//! Producer-path filtering and dispatch for [`FemtoLogger`].
//!
//! These helpers keep the hot logging path focused and separate it from the
//! dispatch pool. Nothing here waits: a record is either handed to the shard or
//! counted as dropped, so a slow handler slows the loggers sharing its shard
//! rather than the code doing the logging.

use std::sync::Arc;

use crate::filters::FilterContext;
use crate::handler::FemtoHandlerTrait;
use crate::level::FemtoLevel;
use crate::log_context;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::manager;

mod admission;

#[cfg(feature = "python")]
mod context_capture;

mod handler_filters;

#[cfg(all(test, not(loom)))]
use super::HandlerAttachment;
use super::{FemtoLogger, QueuedRecord};

impl FemtoLogger {
    /// Returns the logger's name.
    ///
    /// The bridge adapters build their own records so the core logger never has
    /// to name a record's origin; building one needs the name the producer
    /// would otherwise apply. Only those adapters ask, so a build with neither
    /// bridge would otherwise carry this as dead code under `-D warnings`.
    #[inline]
    #[cfg(any(feature = "log-compat", feature = "tracing-compat", test))]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    /// Core logging logic shared between Python and Rust APIs.
    ///
    /// Checks level threshold and filters, formats the record, and dispatches
    /// to handlers. Returns `Some(formatted_message)` if the record was logged,
    /// or `None` if it was filtered out or the logger is disabled.
    ///
    /// The disabled check lives here as well as in the public entry points
    /// because this is the funnel every producer path reaches.
    pub(super) fn log_record(&self, record: FemtoLogRecord) -> Option<String> {
        self.log_record_with_capture(record, |_| {})
    }

    /// Log `record`, running `capture` once the deferred-capture prefix accepts.
    ///
    /// Filters are evaluated in their original order. The leading run of
    /// filters that cannot inspect traceback data runs first; if every one of
    /// them accepts, `capture` attaches the payloads and the remaining filters
    /// run. Capture is therefore skipped for any record rejected by that
    /// prefix, while a callback filter still sees the payload it may inspect.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let record = FemtoLogRecord::new("app", FemtoLevel::Info, "hello");
    /// let logged = logger.log_record_with_capture(record, |record| {
    ///     record.set_exception_payload(captured);
    /// });
    /// ```
    pub(super) fn log_record_with_capture<F>(
        &self,
        mut record: FemtoLogRecord,
        capture: F,
    ) -> Option<String>
    where
        F: FnOnce(&mut FemtoLogRecord),
    {
        if self.refuse_if_disabled() {
            return None;
        }
        let threshold = self.level.load(std::sync::atomic::Ordering::Relaxed);
        if u8::from(record.level()) < threshold {
            return None;
        }

        if !self.apply_filters_with_capture(&mut record, capture) {
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
        if self.refuse_if_disabled() {
            return None;
        }
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
        metadata: RecordMetadata,
    ) -> Option<String> {
        self.log_prebuilt(FemtoLogRecord::with_metadata(
            &self.name, level, message, metadata,
        ))
    }

    /// Log a record the caller has already assembled.
    ///
    /// The bridge adapters build their records themselves, which keeps any
    /// record-origin vocabulary out of this module: a record's origin is
    /// attached before it arrives, and every stage from here on carries it
    /// unchanged through filtering, the handler-filter snapshot and parent
    /// propagation.
    pub(crate) fn log_prebuilt(&self, mut record: FemtoLogRecord) -> Option<String> {
        if self.refuse_if_disabled() {
            return None;
        }
        // Context admission runs before the level gate so a reserved or
        // over-limit field is counted even when the record's level is disabled.
        let merged_key_values =
            match log_context::merge_context_values(&record.metadata().key_values) {
                Ok(merged_key_values) => merged_key_values,
                Err(err) => {
                    self.context_dropped_records
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    eprintln!("FemtoLogger: dropping record due to invalid context payload: {err}");
                    return None;
                }
            };
        if !self.is_enabled_for(record.level()) {
            return None;
        }
        record.metadata_mut().key_values = merged_key_values;
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
        if self.refuse_if_disabled() || !self.is_enabled_for(level) {
            return None;
        }
        let record = FemtoLogRecord::with_metadata(&self.name, level, message, metadata);
        self.log_record(record)
    }

    /// Refuse and count a record because configuration disabled this logger.
    ///
    /// Returns `true` when the logger is disabled, after incrementing
    /// [`Self::disabled_dropped_records`]. Every producer entry point consults
    /// this before its level check, so a disabled logger's records never reach
    /// filters, formatting or dispatch, and the counter records exactly the
    /// records that rejection discarded.
    pub(crate) fn refuse_if_disabled(&self) -> bool {
        let is_disabled = self.is_disabled();
        if is_disabled {
            self.disabled_dropped_records
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        is_disabled
    }

    /// Return whether `level` is enabled for this logger.
    ///
    /// A disabled logger reports every level as disabled, so callers that
    /// guard work with this check skip it entirely.
    pub(crate) fn is_enabled_for(&self, level: FemtoLevel) -> bool {
        !self.is_disabled()
            && u8::from(level) >= self.level.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Clone the level handle so the manager can index it without Python.
    #[cfg(feature = "log-compat")]
    pub(crate) fn level_handle(&self) -> std::sync::Arc<std::sync::atomic::AtomicU8> {
        std::sync::Arc::clone(&self.level)
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
        Self::level_from_raw(self.level.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Decode the logger's validated atomic representation.
    pub(crate) fn level_from_raw(level: u8) -> FemtoLevel {
        match level {
            0 => FemtoLevel::Trace,
            1 => FemtoLevel::Debug,
            2 => FemtoLevel::Info,
            3 => FemtoLevel::Warn,
            4 => FemtoLevel::Error,
            _ => FemtoLevel::Critical,
        }
    }

    /// Apply the configured filter chain, running `capture` at the split point.
    ///
    /// Filters keep their configured order and are evaluated one at a time.
    /// Capture runs immediately before the first filter that can inspect
    /// traceback data, or after the last filter when none can. A record rejected
    /// earlier therefore pays neither for capture nor for the checks that would
    /// have found the split. One [`FilterContext`] is shared by the whole chain,
    /// so a Python record view built by one filter is reused by the next. A
    /// callback failure counts as rejection.
    fn apply_filters_with_capture<F>(&self, record: &mut FemtoLogRecord, capture: F) -> bool
    where
        F: FnOnce(&mut FemtoLogRecord),
    {
        let filters = self.filters.read().clone();
        let mut context = FilterContext::default();
        // Capture belongs immediately before the first filter that can read
        // traceback data. Splitting there lets both halves use the one segment
        // evaluator, so the decision, rejection, failure and enrichment rules
        // have a single implementation.
        let split = filters
            .iter()
            .position(|filter| filter.inspects_traceback())
            .unwrap_or(filters.len());
        let (before, after) = filters.split_at(split);
        if !matches!(
            handler_filters::apply_filter_segment(record, before, &mut context),
            handler_filters::FilterChainOutcome::Accepted
        ) {
            return false;
        }
        capture(record);
        matches!(
            handler_filters::apply_filter_segment(record, after, &mut context),
            handler_filters::FilterChainOutcome::Accepted
        )
    }

    /// Return whether this logger forwards records to its configured parent.
    fn should_propagate_to_parent(&self) -> bool {
        self.propagate.load(std::sync::atomic::Ordering::SeqCst) && self.parent.is_some()
    }

    /// Forward a record to the parent logger when it remains registered.
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

    /// Queue a record for this logger's local handlers on its dispatch shard.
    fn send_to_local_handlers(&self, record: FemtoLogRecord) {
        let Some((record, handlers)) = self.select_local_handlers(record) else {
            return;
        };
        #[cfg(feature = "tracing-compat")]
        let handler_kind = handler_filters::handler_kind(&handlers);
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
        let queued = Box::new(QueuedRecord {
            record,
            handlers,
            #[cfg(feature = "python")]
            context,
        });
        self.admit_local_record(
            queued,
            #[cfg(feature = "tracing-compat")]
            handler_kind,
        );
    }

    /// Return the filtered record and handlers that are ready for dispatch.
    fn select_local_handlers(
        &self,
        mut record: FemtoLogRecord,
    ) -> Option<(FemtoLogRecord, Vec<Arc<dyn FemtoHandlerTrait>>)> {
        let attachments = self.handlers.read().clone();
        if attachments.is_empty() {
            #[cfg(feature = "tracing-compat")]
            tracing::trace!(
                target: "femtologging::internal",
                operation = "producer_handler_dispatch",
                handler_kind = "none",
                dispatch_outcome = "no_local_handlers",
            );
            return None;
        }
        let handlers = handler_filters::filter_handlers(&mut record, attachments);
        if handlers.is_empty() {
            #[cfg(feature = "tracing-compat")]
            tracing::trace!(
                target: "femtologging::internal",
                operation = "producer_handler_dispatch",
                handler_kind = "none",
                dispatch_outcome = "no_handlers_accepted",
            );
            return None;
        }
        Some((record, handlers))
    }

    /// Exercise the production handler-filter chain without starting a worker.
    #[cfg(all(test, not(loom)))]
    pub(super) fn filter_handlers_for_test(
        record: &mut FemtoLogRecord,
        handlers: Vec<HandlerAttachment>,
    ) -> Vec<std::sync::Arc<dyn crate::handler::FemtoHandlerTrait>> {
        handler_filters::filter_handlers(record, handlers)
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
