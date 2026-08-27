//! Percent-style formatting for records emitted through configured handlers.

use chrono::{DateTime, Utc};

use crate::log_record::FemtoLogRecord;

use super::FemtoFormatter;

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
                .map_or_else(String::new, |value| value.to_string()),
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

    use crate::{level::FemtoLevel, log_record::FemtoLogRecord};

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
}
