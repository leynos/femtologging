//! Field visitors for tracing events and spans.

use std::collections::BTreeMap;
use std::fmt;

use tracing::Event;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};

#[derive(Debug, Default)]
pub(crate) struct CapturedFields {
    pub(crate) message: Option<String>,
    pub(crate) key_values: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct FieldCaptureVisitor {
    message: Option<String>,
    key_values: BTreeMap<String, String>,
    /// If true, treat "message" as a regular field instead of extracting it.
    /// This is used for spans, where "message" is just a structured field.
    preserve_message_field: bool,
}

impl FieldCaptureVisitor {
    fn for_event() -> Self {
        Self {
            preserve_message_field: false,
            ..Default::default()
        }
    }

    fn for_span() -> Self {
        Self {
            preserve_message_field: true,
            ..Default::default()
        }
    }

    fn store(&mut self, field: &Field, value: String) {
        if field.name() == "message" && !self.preserve_message_field {
            self.message = Some(value);
        } else {
            self.key_values.insert(field.name().to_string(), value);
        }
    }

    fn finish(self) -> CapturedFields {
        CapturedFields {
            message: self.message,
            key_values: self.key_values,
        }
    }
}

impl Visit for FieldCaptureVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.store(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.store(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.store(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.store(field, value.to_string());
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.store(field, value.to_string());
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.store(field, value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.store(field, value.to_string());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.store(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.store(field, format!("{value:?}"));
    }

    fn record_bytes(&mut self, field: &Field, value: &[u8]) {
        self.store(field, format!("{value:?}"));
    }
}

pub(crate) fn capture_event(event: &Event<'_>) -> CapturedFields {
    let mut visitor = FieldCaptureVisitor::for_event();
    event.record(&mut visitor);
    visitor.finish()
}

pub(crate) fn capture_attributes(attrs: &Attributes<'_>) -> CapturedFields {
    let mut visitor = FieldCaptureVisitor::for_span();
    attrs.record(&mut visitor);
    visitor.finish()
}

pub(crate) fn capture_record(record: &Record<'_>) -> CapturedFields {
    let mut visitor = FieldCaptureVisitor::for_span();
    record.record(&mut visitor);
    visitor.finish()
}
