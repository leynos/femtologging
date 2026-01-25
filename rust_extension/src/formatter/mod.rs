//! Formatter implementations and adapters bridging Rust and Python callers.
//!
//! Provides the core [`FemtoFormatter`] trait alongside helpers for
//! dynamically dispatched trait objects. When the Python feature is enabled we
//! expose adapters for Python callables so they can participate in Rust
//! logging pipelines safely across threads.

use std::{fmt, sync::Arc};

use crate::log_record::FemtoLogRecord;

mod exception;

#[cfg(feature = "python")]
pub mod python;

pub use exception::{ExceptionFormat, format_exception_payload, format_stack_payload};

/// Trait for formatting log records into strings.
///
/// Implementors must be thread-safe (`Send + Sync`) so formatters can be
/// shared across threads in a logging system.
pub trait FemtoFormatter: Send + Sync {
    /// Format a log record into a string representation.
    fn format(&self, record: &FemtoLogRecord) -> String;
}

/// Shared formatter trait object used across handlers.
#[derive(Clone)]
pub struct SharedFormatter {
    inner: Arc<dyn FemtoFormatter + Send + Sync>,
}

impl SharedFormatter {
    /// Create a shared formatter from an owned formatter implementation.
    pub fn new<F>(formatter: F) -> Self
    where
        F: FemtoFormatter + Send + Sync + 'static,
    {
        let inner: Arc<dyn FemtoFormatter + Send + Sync> = Arc::from(formatter);
        Self { inner }
    }

    /// Wrap an existing shared formatter trait object.
    pub fn from_arc(inner: Arc<dyn FemtoFormatter + Send + Sync>) -> Self {
        Self { inner }
    }

    /// Clone the underlying trait object, incrementing the reference count.
    pub fn clone_arc(&self) -> Arc<dyn FemtoFormatter + Send + Sync> {
        Arc::clone(&self.inner)
    }

    /// Format a log record using the wrapped formatter instance.
    pub fn format(&self, record: &FemtoLogRecord) -> String {
        self.inner.format(record)
    }
}

impl fmt::Debug for SharedFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedFormatter(<dyn FemtoFormatter>)")
    }
}

#[derive(Copy, Clone, Debug)]
pub struct DefaultFormatter;

impl FemtoFormatter for DefaultFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        let mut output = format!(
            "{} [{}] {}",
            record.logger(),
            record.level_str(),
            record.message()
        );

        // Append stack trace if present (before exception for readability)
        if let Some(stack) = record.stack_payload() {
            output.push('\n');
            output.push_str(&format_stack_payload(stack));
        }

        // Append exception info if present
        if let Some(exc) = record.exception_payload() {
            output.push('\n');
            output.push_str(&format_exception_payload(exc));
        }

        output
    }
}

impl FemtoFormatter for Arc<dyn FemtoFormatter + Send + Sync> {
    fn format(&self, record: &FemtoLogRecord) -> String {
        (**self).format(record)
    }
}

impl FemtoFormatter for Box<dyn FemtoFormatter + Send + Sync> {
    fn format(&self, record: &FemtoLogRecord) -> String {
        (**self).format(record)
    }
}

#[cfg(test)]
mod tests {
    //! Tests for formatter implementations.

    use super::*;
    use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
    use crate::level::FemtoLevel;
    use static_assertions::assert_impl_all;

    #[test]
    fn shared_formatter_is_send_sync() {
        assert_impl_all!(SharedFormatter: Send, Sync);
        assert_impl_all!(Arc<dyn FemtoFormatter + Send + Sync>: Send, Sync);
    }

    #[test]
    fn default_formatter_formats_basic_record() {
        let formatter = DefaultFormatter;
        let record = FemtoLogRecord::new("test", FemtoLevel::Info, "hello");
        let output = formatter.format(&record);
        assert_eq!(output, "test [INFO] hello");
    }

    #[test]
    fn default_formatter_includes_exception_payload() {
        let formatter = DefaultFormatter;
        let exception = ExceptionPayload::new("ValueError", "test error");
        let record =
            FemtoLogRecord::new("test", FemtoLevel::Error, "failed").with_exception(exception);

        let output = formatter.format(&record);

        assert!(output.starts_with("test [ERROR] failed\n"));
        assert!(output.contains("Traceback (most recent call last):"));
        assert!(output.contains("ValueError: test error"));
    }

    #[test]
    fn default_formatter_includes_stack_payload() {
        let formatter = DefaultFormatter;
        let frame = StackFrame::new("test.py", 42, "test_func");
        let stack = StackTracePayload::new(vec![frame]);
        let record = FemtoLogRecord::new("test", FemtoLevel::Debug, "debug info").with_stack(stack);

        let output = formatter.format(&record);

        assert!(output.starts_with("test [DEBUG] debug info\n"));
        assert!(output.contains("Stack (most recent call last):"));
        assert!(output.contains("test.py"));
        assert!(output.contains("line 42"));
        assert!(output.contains("test_func"));
    }

    #[test]
    fn default_formatter_includes_both_payloads() {
        let formatter = DefaultFormatter;
        let exception = ExceptionPayload::new("RuntimeError", "runtime issue");
        let frame = StackFrame::new("main.py", 10, "main");
        let stack = StackTracePayload::new(vec![frame]);

        let record = FemtoLogRecord::new("app", FemtoLevel::Critical, "crash")
            .with_exception(exception)
            .with_stack(stack);

        let output = formatter.format(&record);

        // Verify structure: message, then stack, then exception
        assert!(output.starts_with("app [CRITICAL] crash\n"));
        let stack_pos = output
            .find("Stack (most recent call last):")
            .expect("stack should be present");
        let traceback_pos = output
            .find("Traceback (most recent call last):")
            .expect("traceback should be present");
        assert!(
            stack_pos < traceback_pos,
            "stack should appear before exception"
        );
        assert!(output.contains("RuntimeError: runtime issue"));
    }
}
