//! Exception and stack trace formatting utilities.
//!
//! Provides human-readable formatting for exception payloads and stack traces
//! following Python's traceback formatting style.

use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};

/// Format a stack trace payload into a human-readable string.
///
/// Follows Python's traceback formatting style.
pub fn format_stack_payload(payload: &StackTracePayload) -> String {
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
pub fn format_exception_payload(payload: &ExceptionPayload) -> String {
    let mut output = format_exception_chain(payload);

    output.push_str(&format_exception_body(payload));

    // Append notes if present
    output.push_str(&format_exception_notes(&payload.notes));

    // Handle exception groups
    output.push_str(&format_exception_group(&payload.exceptions));

    output
}

/// Format a single stack frame into a human-readable string.
pub fn format_stack_frame(frame: &StackFrame) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn format_deep_exception_chain_no_stack_overflow() {
        // Build a 100-level cause chain and format it
        let mut current = ExceptionPayload::new("BaseError", "root cause");
        for i in 1..100 {
            let mut wrapper = ExceptionPayload::new(format!("Error{i}"), format!("level {i}"));
            wrapper.cause = Some(Box::new(current));
            current = wrapper;
        }

        // This should not stack overflow
        let output = format_exception_payload(&current);

        // Verify output contains markers from different levels
        assert!(output.contains("BaseError: root cause"));
        assert!(output.contains("Error99: level 99"));
        assert!(output.contains("The above exception was the direct cause"));
    }
}
