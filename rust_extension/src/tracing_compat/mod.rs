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

const FALLBACK_EVENT_MESSAGE: &str = "tracing event";

#[derive(Debug, Default)]
struct StoredSpanFields {
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
    fn map_level(level: &Level) -> FemtoLevel {
        match *level {
            Level::TRACE => FemtoLevel::Trace,
            Level::DEBUG => FemtoLevel::Debug,
            Level::INFO => FemtoLevel::Info,
            Level::WARN => FemtoLevel::Warn,
            Level::ERROR => FemtoLevel::Error,
        }
    }

    fn normalize_target(target: &str) -> Cow<'_, str> {
        if target.contains("::") {
            Cow::Owned(target.replace("::", "."))
        } else {
            Cow::Borrowed(target)
        }
    }

    fn should_ignore_target(target: &str) -> bool {
        let normalized = Self::normalize_target(target);
        normalized == "femtologging" || normalized.starts_with("femtologging.")
    }

    fn resolve_logger<'py>(
        py: Python<'py>,
        target: &str,
    ) -> Option<(String, Py<crate::FemtoLogger>)> {
        let normalized = Self::normalize_target(target);
        match manager::get_logger(py, normalized.as_ref()) {
            Ok(logger) => Some((normalized.into_owned(), logger)),
            Err(_) => manager::get_logger(py, "root")
                .ok()
                .map(|logger| ("root".to_string(), logger)),
        }
    }

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
        !Self::should_ignore_target(metadata.target())
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
        if extensions.get_mut::<StoredSpanFields>().is_none() {
            extensions.insert(StoredSpanFields::default());
        }
        let stored = extensions
            .get_mut::<StoredSpanFields>()
            .expect("stored span fields must exist after insertion");
        stored.fields.extend(captured.key_values);
    }

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
