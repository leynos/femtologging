//! Percent-style formatting for records emitted through configured handlers.
//!
//! A [`PercentFormatter`] compiles its format string once, at construction, so
//! rendering a record only walks precompiled literal and directive segments.
//! An unsupported directive keeps its leading `%` as literal text and scanning
//! resumes after that `%`, so embedded escapes and later directives are still
//! processed, matching the documented behaviour for lone percent signs and
//! unrecognized conversions.

mod directives;
#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};

use crate::log_record::FemtoLogRecord;

use super::{FemtoFormatter, format_exception_payload, format_stack_payload};
use directives::{Conversion, Segment, compile};

/// Fallback timestamp representation used when no date format is supplied or valid.
const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// A percent-style formatter compatible with the common logging field syntax.
#[derive(Clone, Debug)]
pub struct PercentFormatter {
    /// Original format text, retained for output sizing and diagnostics.
    format: String,
    /// Literal and directive segments compiled once at construction.
    segments: Vec<Segment>,
    /// Optional chrono pattern for the standard `asctime` field.
    datefmt: Option<String>,
}

impl PercentFormatter {
    /// Create a formatter using `format` and an optional timestamp format.
    ///
    /// The format string is parsed once here. An unsupported directive is not
    /// rejected: its `%` becomes literal text and scanning resumes after it,
    /// so a malformed format degrades to readable output instead of failing
    /// configuration.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let formatter = PercentFormatter::new("%(levelname)s: %(message)s", None);
    /// ```
    pub fn new(format: impl Into<String>, datefmt: Option<String>) -> Self {
        let format = format.into();
        let segments = compile(&format);
        Self {
            format,
            segments,
            datefmt,
        }
    }

    /// Render a resolved field using the requested conversion.
    fn format_directive(
        &self,
        record: &FemtoLogRecord,
        field: &str,
        conversion: Conversion,
    ) -> String {
        conversion.render(self.field_value(record, field))
    }

    /// Resolve a standard logging field or a structured metadata value.
    fn field_value(&self, record: &FemtoLogRecord, name: &str) -> Option<String> {
        match name {
            "name" => Some(record.logger().to_owned()),
            "levelname" => Some(record.level_str().to_owned()),
            "levelno" => Some(u8::from(record.level()).to_string()),
            "message" => Some(record.message().to_owned()),
            "asctime" => Some(self.asctime(record)),
            _ => record.metadata().key_values.get(name).cloned(),
        }
    }

    /// Format the record timestamp, falling back if the configured pattern is invalid.
    fn asctime(&self, record: &FemtoLogRecord) -> String {
        let timestamp: DateTime<Utc> = record.metadata().timestamp.into();
        let datefmt = self.datefmt.as_deref().unwrap_or(DEFAULT_DATE_FORMAT);
        let mut formatted = String::new();
        std::fmt::write(
            &mut formatted,
            format_args!("{}", timestamp.format(datefmt)),
        )
        .map_or_else(
            // A caller-supplied format error must not terminate a handler worker.
            |_| timestamp.format(DEFAULT_DATE_FORMAT).to_string(),
            |_| formatted,
        )
    }
}

impl FemtoFormatter for PercentFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        let mut output = String::with_capacity(self.format.len());
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => output.push_str(text),
                Segment::Directive { field, conversion } => {
                    output.push_str(&self.format_directive(record, field, *conversion));
                }
            }
        }

        if let Some(stack) = record.stack_payload() {
            output.push('\n');
            output.push_str(&format_stack_payload(stack));
        }
        if let Some(exception) = record.exception_payload() {
            output.push('\n');
            output.push_str(&format_exception_payload(exception));
        }
        output
    }
}
