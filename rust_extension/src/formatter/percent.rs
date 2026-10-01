//! Percent-style formatting for records emitted through configured handlers.

use chrono::{DateTime, Utc};

use crate::log_record::FemtoLogRecord;

use super::{FemtoFormatter, format_exception_payload, format_stack_payload};

const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// A percent-style formatter compatible with the common logging field syntax.
#[derive(Clone, Debug)]
pub struct PercentFormatter {
    format: String,
    datefmt: Option<String>,
}

impl PercentFormatter {
    /// Create a formatter using `format` and an optional timestamp format.
    pub fn new(format: impl Into<String>, datefmt: Option<String>) -> Self {
        Self {
            format: format.into(),
            datefmt,
        }
    }

    fn format_directive(&self, record: &FemtoLogRecord, name: &str, kind: char) -> String {
        let value = self.field_value(record, name);
        match kind {
            's' => value.unwrap_or_default(),
            'd' => value
                .and_then(|value| value.parse::<i64>().ok())
                .map_or_else(String::new, |value| value.to_string()),
            'f' => value
                .and_then(|value| value.parse::<f64>().ok())
                .map_or_else(String::new, |value| format!("{value:.6}")),
            _ => String::new(),
        }
    }

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

    fn asctime(&self, record: &FemtoLogRecord) -> String {
        let timestamp: DateTime<Utc> = record.metadata().timestamp.into();
        timestamp
            .format(self.datefmt.as_deref().unwrap_or(DEFAULT_DATE_FORMAT))
            .to_string()
    }
}

impl FemtoFormatter for PercentFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        let mut output = String::with_capacity(self.format.len());
        let mut remaining = self.format.as_str();

        while let Some(percent_index) = remaining.find('%') {
            output.push_str(&remaining[..percent_index]);
            remaining = &remaining[percent_index + 1..];

            if let Some(rest) = remaining.strip_prefix('%') {
                output.push('%');
                remaining = rest;
                continue;
            }

            let Some(field) = remaining.strip_prefix('(') else {
                output.push('%');
                continue;
            };
            let Some(close_index) = field.find(')') else {
                output.push('%');
                continue;
            };
            let name = &field[..close_index];
            let kind_start = close_index + 1;
            let Some(kind) = field[kind_start..].chars().next() else {
                output.push('%');
                continue;
            };
            if !matches!(kind, 's' | 'd' | 'f') {
                output.push('%');
                continue;
            }

            output.push_str(&self.format_directive(record, name, kind));
            remaining = &field[kind_start + kind.len_utf8()..];
        }

        output.push_str(remaining);
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

#[cfg(test)]
mod tests {
    //! Tests for percent-style record formatting.

    use std::{
        collections::BTreeMap,
        time::{Duration, UNIX_EPOCH},
    };

    use proptest::prelude::*;

    use crate::{
        exception_schema::{ExceptionPayload, StackFrame, StackTracePayload},
        formatter::{format_exception_payload, format_stack_payload},
        level::FemtoLevel,
        log_record::FemtoLogRecord,
    };

    use super::{FemtoFormatter, PercentFormatter};

    #[test]
    fn formats_standard_and_structured_fields() {
        let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "inside");
        record.metadata_mut().timestamp = UNIX_EPOCH + Duration::from_secs(0);
        record.metadata_mut().key_values =
            BTreeMap::from([("correlation_id".to_owned(), "abc123".to_owned())]);

        let output = PercentFormatter::new(
            "%(name)s %(levelname)s %(levelno)d %(message)s %(correlation_id)s %(missing)s %(asctime)s",
            Some("%Y".to_owned()),
        )
        .format(&record);

        assert_eq!(output, "probe INFO 2 inside abc123  1970");
    }

    #[test]
    fn formats_float_with_six_fractional_digits_by_default() {
        let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "inside");
        record.metadata_mut().key_values = BTreeMap::from([("ratio".to_owned(), "1.5".to_owned())]);

        let output = PercentFormatter::new("%(ratio)f", None).format(&record);

        assert_eq!(output, "1.500000");
    }

    #[test]
    fn appends_stack_and_exception_payloads_after_formatted_text() {
        let exception = ExceptionPayload::new("ValueError", "invalid value");
        let stack = StackTracePayload::new(vec![StackFrame::new("app.py", 12, "run")]);
        let record = FemtoLogRecord::new("probe", FemtoLevel::Error, "failed")
            .with_stack(stack.clone())
            .with_exception(exception.clone());

        let output = PercentFormatter::new("%(message)s", None).format(&record);
        let expected = format!(
            "failed\n{}\n{}",
            format_stack_payload(&stack),
            format_exception_payload(&exception),
        );

        assert_eq!(output, expected);
    }

    proptest! {
        #[test]
        fn formats_generated_tokens_against_reference_output(
            tokens in prop::collection::vec(prop_oneof![
                "[a-zA-Z0-9 _-]{0,8}".prop_map(|literal| (literal.clone(), literal)),
                Just(("%(message)s".to_owned(), "hello".to_owned())),
                Just(("%(levelno)d".to_owned(), "2".to_owned())),
                Just(("%(ratio)f".to_owned(), "1.500000".to_owned())),
                Just(("%(missing)s".to_owned(), String::new())),
                Just(("%%".to_owned(), "%".to_owned())),
            ], 0..16),
            malformed in prop_oneof![
                Just("%".to_owned()),
                Just("%(field".to_owned()),
                Just("%(message)q".to_owned()),
            ]
        ) {
            let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");
            record.metadata_mut().key_values = BTreeMap::from([
                ("ratio".to_owned(), "1.5".to_owned()),
            ]);
            let format = tokens
                .iter()
                .map(|(format, _)| format.as_str())
                .collect::<Vec<_>>()
                .join("|")
                + "|"
                + &malformed;
            let expected = tokens
                .iter()
                .map(|(_, output)| output.as_str())
                .collect::<Vec<_>>()
                .join("|")
                + "|"
                + &malformed;

            prop_assert_eq!(PercentFormatter::new(format, None).format(&record), expected);
        }

        #[test]
        fn arbitrary_percent_sequences_do_not_panic(
            format in prop::collection::vec(any::<char>(), 0..128)
                .prop_map(|characters| characters.into_iter().collect::<String>())
        ) {
            let record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");
            let _ = PercentFormatter::new(format, None).format(&record);
        }
    }
}
