//! Formatter implementations and adapters bridging Rust and Python callers.
//!
//! Provides the core [`FemtoFormatter`] trait alongside helpers for
//! dynamically dispatched trait objects. When the Python feature is enabled we
//! expose adapters for Python callables so they can participate in Rust
//! logging pipelines safely across threads.

use std::{fmt, sync::Arc};

use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
use crate::log_record::FemtoLogRecord;

#[cfg(feature = "python")]
pub mod python;

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
        let mut output = format!("{} [{}] {}", record.logger, record.level, record.message);

        // Append stack trace if present (before exception for readability)
        if let Some(ref stack) = record.stack_payload {
            output.push('\n');
            output.push_str(&format_stack_payload(stack));
        }

        // Append exception info if present
        if let Some(ref exc) = record.exception_payload {
            output.push('\n');
            output.push_str(&format_exception_payload(exc));
        }

        output
    }
}

/// Format a stack trace payload into a human-readable string.
///
/// Follows Python's traceback formatting style.
fn format_stack_payload(payload: &StackTracePayload) -> String {
    let mut output = String::from("Stack (most recent call last):\n");
    for frame in &payload.frames {
        output.push_str(&format_stack_frame(frame));
    }
    output
}

/// Format exception chaining (cause or context) if present.
///
/// Returns the formatted chain output with appropriate separator message.
fn format_exception_chain(payload: &ExceptionPayload) -> String {
    if let Some(ref cause) = payload.cause {
        let mut output = format_exception_payload(cause);
        output
            .push_str("\nThe above exception was the direct cause of the following exception:\n\n");
        output
    } else if let Some(ref context) = payload.context
        && !payload.suppress_context
    {
        let mut output = format_exception_payload(context);
        output
            .push_str("\nDuring handling of the above exception, another exception occurred:\n\n");
        output
    } else {
        String::new()
    }
}

/// Format the exception header line (module, type, and message).
fn format_exception_header(payload: &ExceptionPayload) -> String {
    if let Some(ref module) = payload.module {
        format!("{}.{}: {}\n", module, payload.type_name, payload.message)
    } else {
        format!("{}: {}\n", payload.type_name, payload.message)
    }
}

/// Format exception notes as indented lines.
fn format_exception_notes(notes: &[String]) -> String {
    let mut output = String::new();
    for note in notes {
        output.push_str(&format!("  {}\n", note));
    }
    output
}

/// Format exception groups with indentation.
fn format_exception_group(exceptions: &[ExceptionPayload]) -> String {
    if exceptions.is_empty() {
        return String::new();
    }

    let mut output = String::from("  |\n");
    for (i, nested) in exceptions.iter().enumerate() {
        output.push_str(&format!("  +---- [{}] ", i + 1));
        let nested_str = format_exception_payload(nested);
        // Indent nested exception output
        for line in nested_str.lines() {
            output.push_str(&format!("  |     {}\n", line));
        }
    }
    output
}

/// Format the traceback header, frames, and exception header.
fn format_exception_body(payload: &ExceptionPayload) -> String {
    let mut output = String::from("Traceback (most recent call last):\n");
    for frame in &payload.frames {
        output.push_str(&format_stack_frame(frame));
    }
    output.push_str(&format_exception_header(payload));
    output
}

/// Format an exception payload into a human-readable string.
///
/// Handles exception chaining and follows Python's traceback formatting style.
fn format_exception_payload(payload: &ExceptionPayload) -> String {
    let mut output = format_exception_chain(payload);

    output.push_str(&format_exception_body(payload));

    // Append notes if present
    output.push_str(&format_exception_notes(&payload.notes));

    // Handle exception groups
    output.push_str(&format_exception_group(&payload.exceptions));

    output
}

/// Format a single stack frame into a human-readable string.
fn format_stack_frame(frame: &StackFrame) -> String {
    let mut output = format!(
        "  File \"{}\", line {}, in {}\n",
        frame.filename, frame.lineno, frame.function
    );

    if let Some(ref source) = frame.source_line {
        let trimmed = source.trim_start();
        let trimmed_end = trimmed.trim_end();
        if !trimmed_end.is_empty() {
            output.push_str(&format!("    {}\n", trimmed_end));

            // Add column indicators if available (Python 3.11+)
            // Adjust for leading whitespace that was trimmed
            if let (Some(colno), Some(end_colno)) = (frame.colno, frame.end_colno) {
                // Calculate how many leading chars were trimmed
                let leading_trimmed = source.len() - trimmed.len();
                // Adjust column positions (colno/end_colno are 1-indexed)
                let col_start = (colno.saturating_sub(1) as usize).saturating_sub(leading_trimmed);
                let col_end =
                    (end_colno.saturating_sub(1) as usize).saturating_sub(leading_trimmed);
                let underline_len = col_end.saturating_sub(col_start).max(1);
                output.push_str(&format!(
                    "    {}{}\n",
                    " ".repeat(col_start),
                    "^".repeat(underline_len)
                ));
            }
        }
    }

    output
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
        let mut record = FemtoLogRecord::new("test", FemtoLevel::Error, "failed");
        record.exception_payload = Some(exception);

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
        let mut record = FemtoLogRecord::new("test", FemtoLevel::Debug, "debug info");
        record.stack_payload = Some(stack);

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

        let mut record = FemtoLogRecord::new("app", FemtoLevel::Critical, "crash");
        record.exception_payload = Some(exception);
        record.stack_payload = Some(stack);

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

    #[test]
    fn format_stack_frame_with_source_line() {
        let mut frame = StackFrame::new("example.py", 5, "do_something");
        frame.source_line = Some("    result = calculate()".to_string());

        let output = format_stack_frame(&frame);

        assert!(output.contains("example.py"));
        assert!(output.contains("line 5"));
        assert!(output.contains("do_something"));
        assert!(output.contains("result = calculate()"));
    }

    #[test]
    fn format_stack_frame_with_column_indicators() {
        let mut frame = StackFrame::new("test.py", 10, "func");
        frame.source_line = Some("    x = foo()".to_string());
        frame.colno = Some(9); // 1-indexed, pointing to 'foo'
        frame.end_colno = Some(14); // end of 'foo()'

        let output = format_stack_frame(&frame);

        // Should have underline indicators
        assert!(output.contains("^"));
    }

    #[test]
    fn format_exception_with_cause_chain() {
        let cause = ExceptionPayload::new("OSError", "file not found");
        let mut effect = ExceptionPayload::new("RuntimeError", "operation failed");
        effect.cause = Some(Box::new(cause));

        let output = format_exception_payload(&effect);

        assert!(output.contains("OSError: file not found"));
        assert!(output.contains("RuntimeError: operation failed"));
        assert!(output.contains("The above exception was the direct cause"));
    }

    #[test]
    fn format_exception_with_context_chain() {
        let context = ExceptionPayload::new("ValueError", "invalid input");
        let mut effect = ExceptionPayload::new("TypeError", "type mismatch");
        effect.context = Some(Box::new(context));

        let output = format_exception_payload(&effect);

        assert!(output.contains("ValueError: invalid input"));
        assert!(output.contains("TypeError: type mismatch"));
        assert!(output.contains("During handling of the above exception"));
    }

    #[test]
    fn format_exception_with_module() {
        let mut exception = ExceptionPayload::new("CustomError", "custom message");
        exception.module = Some("myapp.errors".to_string());

        let output = format_exception_payload(&exception);

        assert!(output.contains("myapp.errors.CustomError: custom message"));
    }

    #[test]
    fn format_exception_with_notes() {
        let mut exception = ExceptionPayload::new("ValueError", "bad value");
        exception.notes = vec!["Note 1".to_string(), "Note 2".to_string()];

        let output = format_exception_payload(&exception);

        assert!(output.contains("  Note 1"));
        assert!(output.contains("  Note 2"));
    }

    #[test]
    fn format_exception_group() {
        let nested1 = ExceptionPayload::new("ValueError", "value error");
        let nested2 = ExceptionPayload::new("TypeError", "type error");
        let mut group = ExceptionPayload::new("ExceptionGroup", "multiple errors");
        group.exceptions = vec![nested1, nested2];

        let output = format_exception_payload(&group);

        assert!(output.contains("[1]"));
        assert!(output.contains("[2]"));
        assert!(output.contains("ValueError: value error"));
        assert!(output.contains("TypeError: type error"));
    }
}
