//! Compatibility bridge for the Rust `tracing` ecosystem.
//!
//! This module provides [`FemtoTracingLayer`], a
//! `tracing_subscriber::Layer` implementation that converts tracing events
//! into [`crate::FemtoLogRecord`] values and routes them through the existing
//! femtologging logger and handler pipeline.

use std::borrow::Cow;
use std::collections::BTreeMap;

use pyo3::prelude::*;
use tracing::{Event, Level, Subscriber, span::Attributes, span::Id, span::Record};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use crate::level::FemtoLevel;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::manager;

pub mod python;
mod visitor;

#[cfg(test)]
mod tests;

/// Message used when a tracing event has no `message` field.
const FALLBACK_EVENT_MESSAGE: &str = "tracing event";

/// Span fields captured at creation or update until an event consumes them.
#[derive(Debug, Default)]
struct StoredSpanFields {
    /// Latest string representations keyed by tracing field name.
    fields: BTreeMap<String, String>,
}

/// A `tracing_subscriber::Layer` that forwards tracing events to femtologging.
#[derive(Clone, Copy, Debug, Default)]
pub struct FemtoTracingLayer;

/// Construct a tracing layer that forwards events into femtologging.
#[must_use]
pub fn layer() -> FemtoTracingLayer {
    FemtoTracingLayer
}

impl FemtoTracingLayer {
    /// Convert tracing's five levels to the corresponding femtologging level.
    fn map_level(level: &Level) -> FemtoLevel {
        match *level {
            Level::TRACE => FemtoLevel::Trace,
            Level::DEBUG => FemtoLevel::Debug,
            Level::INFO => FemtoLevel::Info,
            Level::WARN => FemtoLevel::Warn,
            Level::ERROR => FemtoLevel::Error,
        }
    }

    /// Convert Rust's `module::target` spelling to femtologging's dotted name.
    fn normalize_target(target: &str) -> Cow<'_, str> {
        if target.contains("::") {
            Cow::Owned(target.replace("::", "."))
        } else {
            Cow::Borrowed(target)
        }
    }

    /// Identify the bridge's own targets so it does not recursively log itself.
    fn should_ignore_target(target: &str) -> bool {
        let normalized = Self::normalize_target(target);
        normalized == "femtologging" || normalized.starts_with("femtologging.")
    }

    /// Resolve a normalized target, falling back to the root logger if absent.
    fn resolve_logger<'a, 'py>(
        py: Python<'py>,
        target: &'a str,
    ) -> Option<(Cow<'a, str>, Py<crate::FemtoLogger>)> {
        let normalized = Self::normalize_target(target);
        match manager::get_logger(py, normalized.as_ref()) {
            Ok(logger) => Some((normalized, logger)),
            Err(_) => manager::get_logger(py, "root")
                .ok()
                .map(|logger| (Cow::Borrowed("root"), logger)),
        }
    }

    /// Build record source metadata, then add the event's active span fields.
    fn build_record_metadata<S>(
        event: &Event<'_>,
        ctx: Context<'_, S>,
        key_values: BTreeMap<String, String>,
    ) -> RecordMetadata
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        let mut metadata = RecordMetadata {
            module_path: event
                .metadata()
                .module_path()
                .unwrap_or_default()
                .to_string(),
            filename: event.metadata().file().unwrap_or_default().to_string(),
            line_number: event.metadata().line().unwrap_or(0),
            key_values,
            ..Default::default()
        };
        Self::merge_span_context(&mut metadata.key_values, ctx, event);
        metadata
    }

    /// Add root-to-leaf span names and stored fields with `span.<depth>` prefixes.
    fn merge_span_context<S>(
        key_values: &mut BTreeMap<String, String>,
        ctx: Context<'_, S>,
        event: &Event<'_>,
    ) where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };

        for (depth, span) in scope.from_root().enumerate() {
            let prefix = format!("span.{depth}");
            key_values.insert(format!("{prefix}.name"), span.metadata().name().to_string());

            if let Some(stored) = span.extensions().get::<StoredSpanFields>() {
                for (key, value) in &stored.fields {
                    key_values.insert(format!("{prefix}.{key}"), value.clone());
                }
            }
        }
    }

    /// Build a stable fallback message from captured fields when no message exists.
    fn fallback_message(key_values: &BTreeMap<String, String>) -> String {
        if key_values.is_empty() {
            return FALLBACK_EVENT_MESSAGE.to_string();
        }

        let joined = key_values
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{FALLBACK_EVENT_MESSAGE} ({joined})")
    }
}

impl<S> Layer<S> for FemtoTracingLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        if Self::should_ignore_target(metadata.target()) {
            return false;
        }

        // Check logger-level filtering to avoid evaluating field expressions for disabled events.
        let level = Self::map_level(metadata.level());
        Python::attach(|py| {
            let Some((_, logger)) = Self::resolve_logger(py, metadata.target()) else {
                return false;
            };
            logger.borrow(py).is_enabled_for(level)
        })
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };

        let captured = visitor::capture_attributes(attrs);
        span.extensions_mut().insert(StoredSpanFields {
            fields: captured.key_values,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };

        let captured = visitor::capture_record(values);
        let mut extensions = span.extensions_mut();
        if let Some(stored) = extensions.get_mut::<StoredSpanFields>() {
            stored.fields.extend(captured.key_values);
        } else {
            extensions.insert(StoredSpanFields {
                fields: captured.key_values,
            });
        }
    }

    /// Process a tracing event and dispatch it to the femtologging handler pipeline.
    ///
    /// # Performance Characteristics
    ///
    /// This method acquires the Python GIL (Global Interpreter Lock) **for every tracing event**
    /// to resolve the logger, check level filtering, and dispatch the record. While
    /// [`visitor::capture_event`], [`Self::resolve_logger`], [`Self::build_record_metadata`],
    /// and [`FemtoLogRecord::with_metadata`] run before or around the GIL boundary, the per-event
    /// GIL acquisition can introduce latency in high-throughput scenarios.
    ///
    /// ## Mitigation Strategies
    ///
    /// For applications emitting thousands of tracing events per second, consider these approaches
    /// to reduce per-event GIL overhead:
    ///
    /// - **Buffering in Rust**: Collect event data (target, level, message, fields) in a
    ///   lock-free queue or bounded channel on the Rust side, then flush to Python periodically
    ///   or in a dedicated worker thread. This amortizes GIL acquisition across batches of events.
    ///
    /// - **Lock-Free Queues**: Use structures like `crossbeam::queue::ArrayQueue` or
    ///   `crossbeam::channel` with a background thread that calls `Python::attach` once per batch,
    ///   decoupling event capture from Python dispatch.
    ///
    /// - **Async Dispatch**: Enqueue serialized event payloads and flush them asynchronously,
    ///   allowing the hot path to return immediately after pushing to the queue.
    ///
    /// ## Trade-Offs
    ///
    /// Buffering introduces memory overhead and may delay event visibility in logs. For most
    /// applications, the per-event GIL acquisition is acceptable. Profile your workload to
    /// determine if batching is necessary.
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if Self::should_ignore_target(event.metadata().target()) {
            return;
        }

        let captured = visitor::capture_event(event);
        let message = captured
            .message
            .clone()
            .unwrap_or_else(|| Self::fallback_message(&captured.key_values));
        let level = Self::map_level(event.metadata().level());

        Python::attach(|py| {
            let Some((logger_name, logger)) = Self::resolve_logger(py, event.metadata().target())
            else {
                return;
            };

            if !logger.borrow(py).is_enabled_for(level) {
                return;
            }

            let metadata = Self::build_record_metadata(event, ctx, captured.key_values);
            let record = FemtoLogRecord::with_metadata(&logger_name, level, &message, metadata);
            logger.borrow(py).dispatch_record(record);
        });
    }
}
