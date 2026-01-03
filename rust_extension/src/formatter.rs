//! Formatter implementations and adapters bridging Rust and Python callers.
//!
//! Provides the core [`FemtoFormatter`] trait alongside helpers for
//! dynamically dispatched trait objects. When the Python feature is enabled we
//! expose adapters for Python callables so they can participate in Rust
//! logging pipelines safely across threads.

use std::{fmt, sync::Arc};

use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
use crate::log_record::FemtoLogRecord;

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

/// Format an exception payload into a human-readable string.
///
/// Handles exception chaining and follows Python's traceback formatting style.
fn format_exception_payload(payload: &ExceptionPayload) -> String {
    let mut output = format_exception_chain(payload);

    // Format the traceback header and frames
    output.push_str("Traceback (most recent call last):\n");
    for frame in &payload.frames {
        output.push_str(&format_stack_frame(frame));
    }

    // Format the exception type and message
    output.push_str(&format_exception_header(payload));

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

#[cfg(feature = "python")]
pub mod python {
    //! Helpers for adapting Python callables into [`FemtoFormatter`] instances.
    use std::sync::{Arc, Mutex};
    use std::time::UNIX_EPOCH;

    use pyo3::{
        exceptions::PyTypeError,
        prelude::*,
        types::{PyDict, PyList, PyString},
    };

    use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
    use crate::{log_record::FemtoLogRecord, python::fq_py_type};

    use super::{FemtoFormatter, SharedFormatter};

    #[derive(Clone)]
    struct PythonFormatter {
        callable: Arc<Mutex<Py<PyAny>>>,
        description: String,
    }

    impl PythonFormatter {
        fn try_new(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
            let description = fq_py_type(obj);
            if let Ok(s) = obj.downcast::<PyString>() {
                let msg = format!(
                    "formatter must be callable or provide a callable format() method (got string: {s})",
                );
                return Err(PyTypeError::new_err(msg));
            }
            let callable = if obj.is_callable() {
                obj.clone().unbind()
            } else {
                let format = obj.getattr("format").map_err(|_| {
                    PyTypeError::new_err(format!(
                        "formatter must be callable or provide a callable format() method (got Python type: {description})",
                    ))
                })?;
                if !format.is_callable() {
                    return Err(PyTypeError::new_err(format!(
                        "formatter.format must be callable (got Python type: {description})",
                    )));
                }
                format.clone().unbind()
            };
            Ok(Self {
                callable: Arc::new(Mutex::new(callable)),
                description,
            })
        }

        fn call(&self, record: &FemtoLogRecord) -> PyResult<String> {
            Python::with_gil(|py| {
                let payload = record_to_dict(py, record)?;
                let callable = {
                    let guard = self
                        .callable
                        .lock()
                        .expect("Python formatter mutex must not be poisoned");
                    guard.clone_ref(py)
                };
                let result = callable.call1(py, (payload,))?;
                result.extract::<String>(py)
            })
        }
    }

    /// Convert a [`FemtoLogRecord`] to a Python dict for use by Python handlers/formatters.
    ///
    /// This function is used by both the Python formatter adapter and the
    /// `handle_record` hook for Python handlers.
    pub fn record_to_dict(py: Python<'_>, record: &FemtoLogRecord) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        dict.set_item("logger", &record.logger)?;
        dict.set_item("level", &record.level)?;
        dict.set_item("message", &record.message)?;
        if let Some(level) = record.parsed_level {
            dict.set_item("levelno", u8::from(level))?;
        }

        let metadata = PyDict::new(py);
        metadata.set_item("module_path", &record.metadata.module_path)?;
        metadata.set_item("filename", &record.metadata.filename)?;
        metadata.set_item("line_number", record.metadata.line_number)?;
        metadata.set_item("thread_name", &record.metadata.thread_name)?;
        metadata.set_item("thread_id", format!("{:?}", record.metadata.thread_id))?;
        let timestamp = record
            .metadata
            .timestamp
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or_default();
        metadata.set_item("timestamp", timestamp)?;

        let kv = PyDict::new(py);
        for (key, value) in &record.metadata.key_values {
            kv.set_item(key, value)?;
        }
        metadata.set_item("key_values", kv)?;
        dict.set_item("metadata", metadata)?;

        // Add exception payload if present
        if let Some(ref exc) = record.exception_payload {
            dict.set_item("exc_info", exception_payload_to_py(py, exc)?)?;
        }

        // Add stack payload if present
        if let Some(ref stack) = record.stack_payload {
            dict.set_item("stack_info", stack_payload_to_py(py, stack)?)?;
        }

        Ok(dict.into())
    }

    /// Convert a `StackFrame` to a Python dict.
    fn stack_frame_to_py(py: Python<'_>, frame: &StackFrame) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        dict.set_item("filename", &frame.filename)?;
        dict.set_item("lineno", frame.lineno)?;
        dict.set_item("function", &frame.function)?;

        if let Some(end_lineno) = frame.end_lineno {
            dict.set_item("end_lineno", end_lineno)?;
        }
        if let Some(colno) = frame.colno {
            dict.set_item("colno", colno)?;
        }
        if let Some(end_colno) = frame.end_colno {
            dict.set_item("end_colno", end_colno)?;
        }
        if let Some(ref source_line) = frame.source_line {
            dict.set_item("source_line", source_line)?;
        }
        if let Some(ref locals) = frame.locals {
            let locals_dict = PyDict::new(py);
            for (k, v) in locals {
                locals_dict.set_item(k, v)?;
            }
            dict.set_item("locals", locals_dict)?;
        }

        Ok(dict.into())
    }

    /// Convert a `StackTracePayload` to a Python dict.
    fn stack_payload_to_py(py: Python<'_>, payload: &StackTracePayload) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        dict.set_item("schema_version", payload.schema_version)?;

        let frames_list = PyList::empty(py);
        for frame in &payload.frames {
            frames_list.append(stack_frame_to_py(py, frame)?)?;
        }
        dict.set_item("frames", frames_list)?;

        Ok(dict.into())
    }

    /// Convert a `&[String]` to a Python list.
    fn string_vec_to_pylist(py: Python<'_>, strings: &[String]) -> PyResult<PyObject> {
        let list = PyList::empty(py);
        for s in strings {
            list.append(s)?;
        }
        Ok(list.into())
    }

    /// Convert a `&[StackFrame]` to a Python list of dicts.
    fn frames_to_pylist(py: Python<'_>, frames: &[StackFrame]) -> PyResult<PyObject> {
        let list = PyList::empty(py);
        for frame in frames {
            list.append(stack_frame_to_py(py, frame)?)?;
        }
        Ok(list.into())
    }

    /// Convert a `&[ExceptionPayload]` to a Python list of dicts.
    fn exceptions_to_pylist(py: Python<'_>, exceptions: &[ExceptionPayload]) -> PyResult<PyObject> {
        let list = PyList::empty(py);
        for exc in exceptions {
            list.append(exception_payload_to_py(py, exc)?)?;
        }
        Ok(list.into())
    }

    /// Set optional fields on the exception payload dict.
    fn set_optional_exception_items(
        py: Python<'_>,
        dict: &Bound<'_, PyDict>,
        payload: &ExceptionPayload,
    ) -> PyResult<()> {
        if let Some(ref module) = payload.module {
            dict.set_item("module", module)?;
        }

        if !payload.args_repr.is_empty() {
            dict.set_item("args_repr", string_vec_to_pylist(py, &payload.args_repr)?)?;
        }

        if !payload.notes.is_empty() {
            dict.set_item("notes", string_vec_to_pylist(py, &payload.notes)?)?;
        }

        if !payload.frames.is_empty() {
            dict.set_item("frames", frames_to_pylist(py, &payload.frames)?)?;
        }

        if let Some(ref cause) = payload.cause {
            dict.set_item("cause", exception_payload_to_py(py, cause)?)?;
        }

        if let Some(ref context) = payload.context {
            dict.set_item("context", exception_payload_to_py(py, context)?)?;
        }

        if payload.suppress_context {
            dict.set_item("suppress_context", true)?;
        }

        if !payload.exceptions.is_empty() {
            dict.set_item("exceptions", exceptions_to_pylist(py, &payload.exceptions)?)?;
        }

        Ok(())
    }

    /// Convert an `ExceptionPayload` to a Python dict.
    fn exception_payload_to_py(py: Python<'_>, payload: &ExceptionPayload) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        dict.set_item("schema_version", payload.schema_version)?;
        dict.set_item("type_name", &payload.type_name)?;
        dict.set_item("message", &payload.message)?;

        set_optional_exception_items(py, &dict, payload)?;

        Ok(dict.into())
    }

    impl FemtoFormatter for PythonFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            match self.call(record) {
                Ok(result) => result,
                Err(err) => Python::with_gil(|py| {
                    err.print(py);
                    format!("<formatter error in {}>", self.description)
                }),
            }
        }
    }

    /// Convert a Python formatter object into a shared [`FemtoFormatter`] (`Arc` trait object).
    pub fn formatter_from_py(obj: &Bound<'_, PyAny>) -> PyResult<SharedFormatter> {
        PythonFormatter::try_new(obj)
            .map(|formatter| SharedFormatter::from_arc(Arc::new(formatter)))
            .map_err(|err| {
                let py = obj.py();
                let context = PyTypeError::new_err(
                    "formatter must be callable or expose a format(record: Mapping) -> str method",
                );
                context.set_cause(py, Some(err));
                context
            })
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
