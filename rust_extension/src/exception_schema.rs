//! Versioned schema for Python exception and stack trace payloads.
//!
//! This module defines structured types that capture exception data from
//! Python's `traceback` module. The schema is versioned to allow evolution
//! without breaking formatters or handlers.
//!
//! # Schema Version
//!
//! The current schema version is [`EXCEPTION_SCHEMA_VERSION`]. Consumers
//! should check this version when deserializing payloads to handle
//! forward/backward compatibility.
//!
//! # Example
//!
//! ```rust
//! use _femtologging_rs::exception_schema::{
//!     ExceptionPayload, StackFrame, EXCEPTION_SCHEMA_VERSION,
//! };
//!
//! let frame = StackFrame {
//!     filename: "example.py".into(),
//!     lineno: 42,
//!     function: "main".into(),
//!     ..Default::default()
//! };
//!
//! let payload = ExceptionPayload {
//!     schema_version: EXCEPTION_SCHEMA_VERSION,
//!     type_name: "ValueError".into(),
//!     message: "invalid input".into(),
//!     frames: vec![frame],
//!     ..Default::default()
//! };
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current schema version for exception payloads.
///
/// Increment this when making breaking changes to the schema structure.
/// Consumers should check this value when deserializing to handle
/// compatibility.
pub const EXCEPTION_SCHEMA_VERSION: u16 = 1;

/// A single frame in a Python stack trace.
///
/// Corresponds to data available from `traceback.FrameSummary` and
/// Python 3.11+ enhanced traceback information.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackFrame {
    /// Source filename where the frame originated.
    pub filename: String,
    /// Line number in the source file.
    pub lineno: u32,
    /// End line number (Python 3.11+, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_lineno: Option<u32>,
    /// Column offset (Python 3.11+, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colno: Option<u32>,
    /// End column offset (Python 3.11+, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_colno: Option<u32>,
    /// Function or method name.
    pub function: String,
    /// Source code line (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_line: Option<String>,
    /// Local variables as string representations (if captured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locals: Option<BTreeMap<String, String>>,
}

/// A standalone stack trace without exception context.
///
/// Used for `stack_info=True` logging where no exception is involved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackTracePayload {
    /// Schema version for forward compatibility.
    pub schema_version: u16,
    /// Stack frames from innermost to outermost.
    pub frames: Vec<StackFrame>,
}

/// Complete structured representation of a Python exception.
///
/// Captures all relevant data from `traceback.TracebackException` including
/// exception chaining (`__cause__`, `__context__`) and exception groups.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionPayload {
    /// Schema version for forward compatibility.
    pub schema_version: u16,
    /// Exception class name (e.g., "ValueError", "KeyError").
    pub type_name: String,
    /// Module path of the exception class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Exception message (from `str(exception)`).
    pub message: String,
    /// String representations of exception constructor arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args_repr: Vec<String>,
    /// Exception notes (`__notes__`, Python 3.11+).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Stack frames from innermost to outermost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<StackFrame>,
    /// Explicit exception cause (`__cause__` from `raise ... from ...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Box<ExceptionPayload>>,
    /// Implicit exception context (`__context__`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Box<ExceptionPayload>>,
    /// Whether implicit context should be suppressed in display.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppress_context: bool,
    /// Nested exceptions (for `ExceptionGroup`, Python 3.11+).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<ExceptionPayload>,
}

impl StackFrame {
    /// Create a new stack frame with required fields.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::StackFrame;
    ///
    /// let frame = StackFrame::new("test.py", 10, "test_func");
    /// assert_eq!(frame.filename, "test.py");
    /// assert_eq!(frame.lineno, 10);
    /// assert_eq!(frame.function, "test_func");
    /// ```
    pub fn new(filename: impl Into<String>, lineno: u32, function: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            lineno,
            function: function.into(),
            ..Default::default()
        }
    }
}

impl StackTracePayload {
    /// Create a new stack trace payload with the current schema version.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{
    ///     StackFrame, StackTracePayload, EXCEPTION_SCHEMA_VERSION,
    /// };
    ///
    /// let frames = vec![StackFrame::new("test.py", 10, "main")];
    /// let payload = StackTracePayload::new(frames);
    /// assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
    /// ```
    pub fn new(frames: Vec<StackFrame>) -> Self {
        Self {
            schema_version: EXCEPTION_SCHEMA_VERSION,
            frames,
        }
    }

    /// Return a new payload with frames filtered by the given predicate.
    ///
    /// Frames for which the predicate returns `true` are included in the
    /// result.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{StackFrame, StackTracePayload};
    ///
    /// let frames = vec![
    ///     StackFrame::new("app.py", 10, "main"),
    ///     StackFrame::new("logging/__init__.py", 20, "info"),
    /// ];
    /// let payload = StackTracePayload::new(frames);
    ///
    /// // Exclude logging frames
    /// let filtered = payload.filter(|f| !f.filename.contains("logging"));
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn filter<F>(&self, predicate: F) -> Self
    where
        F: Fn(&StackFrame) -> bool,
    {
        Self {
            schema_version: self.schema_version,
            frames: self
                .frames
                .iter()
                .filter(|f| predicate(f))
                .cloned()
                .collect(),
        }
    }

    /// Return a new payload with at most `n` frames (most recent).
    ///
    /// Stack frames are typically ordered from oldest to newest. This keeps
    /// the last `n` frames, which are closest to where the exception occurred.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{StackFrame, StackTracePayload};
    ///
    /// let frames = vec![
    ///     StackFrame::new("a.py", 1, "outer"),
    ///     StackFrame::new("b.py", 2, "middle"),
    ///     StackFrame::new("c.py", 3, "inner"),
    /// ];
    /// let payload = StackTracePayload::new(frames);
    ///
    /// let limited = payload.limit(2);
    /// assert_eq!(limited.frames.len(), 2);
    /// assert_eq!(limited.frames[0].filename, "b.py");
    /// ```
    #[must_use]
    pub fn limit(&self, n: usize) -> Self {
        use crate::frame_filter::limit_frames;
        Self {
            schema_version: self.schema_version,
            frames: limit_frames(&self.frames, n),
        }
    }

    /// Return a new payload excluding frames matching filename patterns.
    ///
    /// Patterns are matched as substrings of the filename.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{StackFrame, StackTracePayload};
    ///
    /// let frames = vec![
    ///     StackFrame::new("myapp/main.py", 10, "main"),
    ///     StackFrame::new(".venv/lib/foo.py", 20, "foo"),
    /// ];
    /// let payload = StackTracePayload::new(frames);
    ///
    /// let filtered = payload.exclude_filenames(&[".venv/"]);
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn exclude_filenames(&self, patterns: &[&str]) -> Self {
        use crate::frame_filter::exclude_by_filename;
        Self {
            schema_version: self.schema_version,
            frames: exclude_by_filename(&self.frames, patterns),
        }
    }

    /// Return a new payload excluding frames matching function name patterns.
    ///
    /// Patterns are matched as substrings of the function name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{StackFrame, StackTracePayload};
    ///
    /// let frames = vec![
    ///     StackFrame::new("app.py", 10, "main"),
    ///     StackFrame::new("app.py", 20, "_internal_helper"),
    /// ];
    /// let payload = StackTracePayload::new(frames);
    ///
    /// let filtered = payload.exclude_functions(&["_internal"]);
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn exclude_functions(&self, patterns: &[&str]) -> Self {
        use crate::frame_filter::exclude_by_function;
        Self {
            schema_version: self.schema_version,
            frames: exclude_by_function(&self.frames, patterns),
        }
    }

    /// Return a new payload excluding common logging infrastructure frames.
    ///
    /// Removes frames from femtologging, Python's standard logging module,
    /// and import machinery.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{StackFrame, StackTracePayload};
    ///
    /// let frames = vec![
    ///     StackFrame::new("myapp/main.py", 10, "run"),
    ///     StackFrame::new("femtologging/__init__.py", 50, "info"),
    ///     StackFrame::new("logging/__init__.py", 100, "_log"),
    /// ];
    /// let payload = StackTracePayload::new(frames);
    ///
    /// let filtered = payload.exclude_logging_infrastructure();
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn exclude_logging_infrastructure(&self) -> Self {
        use crate::frame_filter::exclude_logging_infrastructure;
        Self {
            schema_version: self.schema_version,
            frames: exclude_logging_infrastructure(&self.frames),
        }
    }
}

impl ExceptionPayload {
    /// Create a new exception payload with the current schema version.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{
    ///     ExceptionPayload, EXCEPTION_SCHEMA_VERSION,
    /// };
    ///
    /// let payload = ExceptionPayload::new("ValueError", "invalid input");
    /// assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
    /// assert_eq!(payload.type_name, "ValueError");
    /// ```
    pub fn new(type_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            schema_version: EXCEPTION_SCHEMA_VERSION,
            type_name: type_name.into(),
            message: message.into(),
            ..Default::default()
        }
    }

    /// Add an explicit cause to the exception chain.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::ExceptionPayload;
    ///
    /// let cause = ExceptionPayload::new("IOError", "file not found");
    /// let error = ExceptionPayload::new("RuntimeError", "failed")
    ///     .with_cause(cause);
    /// assert!(error.cause.is_some());
    /// ```
    #[must_use]
    pub fn with_cause(mut self, cause: ExceptionPayload) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// Add implicit context to the exception.
    #[must_use]
    pub fn with_context(mut self, context: ExceptionPayload) -> Self {
        self.context = Some(Box::new(context));
        self
    }

    /// Add stack frames to the exception.
    #[must_use]
    pub fn with_frames(mut self, frames: Vec<StackFrame>) -> Self {
        self.frames = frames;
        self
    }

    /// Return a new payload with frames filtered by the given predicate.
    ///
    /// Recursively filters frames in the cause chain, context chain, and
    /// exception groups.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{ExceptionPayload, StackFrame};
    ///
    /// let frames = vec![
    ///     StackFrame::new("app.py", 10, "main"),
    ///     StackFrame::new("logging/__init__.py", 20, "info"),
    /// ];
    /// let payload = ExceptionPayload::new("ValueError", "test")
    ///     .with_frames(frames);
    ///
    /// let filtered = payload.filter_frames(|f| !f.filename.contains("logging"));
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn filter_frames<F>(&self, predicate: F) -> Self
    where
        F: Fn(&StackFrame) -> bool + Clone,
    {
        Self {
            schema_version: self.schema_version,
            type_name: self.type_name.clone(),
            module: self.module.clone(),
            message: self.message.clone(),
            args_repr: self.args_repr.clone(),
            notes: self.notes.clone(),
            frames: self
                .frames
                .iter()
                .filter(|f| predicate(f))
                .cloned()
                .collect(),
            cause: self
                .cause
                .as_ref()
                .map(|c| Box::new(c.filter_frames(predicate.clone()))),
            context: self
                .context
                .as_ref()
                .map(|c| Box::new(c.filter_frames(predicate.clone()))),
            suppress_context: self.suppress_context,
            exceptions: self
                .exceptions
                .iter()
                .map(|e| e.filter_frames(predicate.clone()))
                .collect(),
        }
    }

    /// Return a new payload with at most `n` frames (most recent).
    ///
    /// Recursively limits frames in the cause chain, context chain, and
    /// exception groups.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{ExceptionPayload, StackFrame};
    ///
    /// let frames = vec![
    ///     StackFrame::new("a.py", 1, "outer"),
    ///     StackFrame::new("b.py", 2, "middle"),
    ///     StackFrame::new("c.py", 3, "inner"),
    /// ];
    /// let payload = ExceptionPayload::new("Error", "test").with_frames(frames);
    ///
    /// let limited = payload.limit_frames(2);
    /// assert_eq!(limited.frames.len(), 2);
    /// ```
    #[must_use]
    pub fn limit_frames(&self, n: usize) -> Self {
        use crate::frame_filter::limit_frames;
        Self {
            schema_version: self.schema_version,
            type_name: self.type_name.clone(),
            module: self.module.clone(),
            message: self.message.clone(),
            args_repr: self.args_repr.clone(),
            notes: self.notes.clone(),
            frames: limit_frames(&self.frames, n),
            cause: self.cause.as_ref().map(|c| Box::new(c.limit_frames(n))),
            context: self.context.as_ref().map(|c| Box::new(c.limit_frames(n))),
            suppress_context: self.suppress_context,
            exceptions: self.exceptions.iter().map(|e| e.limit_frames(n)).collect(),
        }
    }

    /// Return a new payload excluding frames matching filename patterns.
    ///
    /// Recursively excludes frames in the cause chain, context chain, and
    /// exception groups.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{ExceptionPayload, StackFrame};
    ///
    /// let frames = vec![
    ///     StackFrame::new("myapp/main.py", 10, "main"),
    ///     StackFrame::new(".venv/lib/foo.py", 20, "foo"),
    /// ];
    /// let payload = ExceptionPayload::new("Error", "test").with_frames(frames);
    ///
    /// let filtered = payload.exclude_filenames(&[".venv/"]);
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn exclude_filenames(&self, patterns: &[&str]) -> Self {
        use crate::frame_filter::exclude_by_filename;
        Self {
            schema_version: self.schema_version,
            type_name: self.type_name.clone(),
            module: self.module.clone(),
            message: self.message.clone(),
            args_repr: self.args_repr.clone(),
            notes: self.notes.clone(),
            frames: exclude_by_filename(&self.frames, patterns),
            cause: self
                .cause
                .as_ref()
                .map(|c| Box::new(c.exclude_filenames(patterns))),
            context: self
                .context
                .as_ref()
                .map(|c| Box::new(c.exclude_filenames(patterns))),
            suppress_context: self.suppress_context,
            exceptions: self
                .exceptions
                .iter()
                .map(|e| e.exclude_filenames(patterns))
                .collect(),
        }
    }

    /// Return a new payload excluding frames matching function name patterns.
    ///
    /// Recursively excludes frames in the cause chain, context chain, and
    /// exception groups.
    #[must_use]
    pub fn exclude_functions(&self, patterns: &[&str]) -> Self {
        use crate::frame_filter::exclude_by_function;
        Self {
            schema_version: self.schema_version,
            type_name: self.type_name.clone(),
            module: self.module.clone(),
            message: self.message.clone(),
            args_repr: self.args_repr.clone(),
            notes: self.notes.clone(),
            frames: exclude_by_function(&self.frames, patterns),
            cause: self
                .cause
                .as_ref()
                .map(|c| Box::new(c.exclude_functions(patterns))),
            context: self
                .context
                .as_ref()
                .map(|c| Box::new(c.exclude_functions(patterns))),
            suppress_context: self.suppress_context,
            exceptions: self
                .exceptions
                .iter()
                .map(|e| e.exclude_functions(patterns))
                .collect(),
        }
    }

    /// Return a new payload excluding common logging infrastructure frames.
    ///
    /// Recursively excludes frames in the cause chain, context chain, and
    /// exception groups.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use _femtologging_rs::exception_schema::{ExceptionPayload, StackFrame};
    ///
    /// let frames = vec![
    ///     StackFrame::new("myapp/main.py", 10, "run"),
    ///     StackFrame::new("femtologging/__init__.py", 50, "info"),
    /// ];
    /// let payload = ExceptionPayload::new("Error", "test").with_frames(frames);
    ///
    /// let filtered = payload.exclude_logging_infrastructure();
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn exclude_logging_infrastructure(&self) -> Self {
        use crate::frame_filter::exclude_logging_infrastructure;
        Self {
            schema_version: self.schema_version,
            type_name: self.type_name.clone(),
            module: self.module.clone(),
            message: self.message.clone(),
            args_repr: self.args_repr.clone(),
            notes: self.notes.clone(),
            frames: exclude_logging_infrastructure(&self.frames),
            cause: self
                .cause
                .as_ref()
                .map(|c| Box::new(c.exclude_logging_infrastructure())),
            context: self
                .context
                .as_ref()
                .map(|c| Box::new(c.exclude_logging_infrastructure())),
            suppress_context: self.suppress_context,
            exceptions: self
                .exceptions
                .iter()
                .map(ExceptionPayload::exclude_logging_infrastructure)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmp_serde::Serializer;
    use rstest::rstest;
    use serde::Serialize;

    #[rstest]
    fn schema_version_is_one() {
        assert_eq!(EXCEPTION_SCHEMA_VERSION, 1);
    }

    #[rstest]
    fn stack_frame_new_sets_required_fields() {
        let frame = StackFrame::new("test.py", 42, "test_func");
        assert_eq!(frame.filename, "test.py");
        assert_eq!(frame.lineno, 42);
        assert_eq!(frame.function, "test_func");
        assert!(frame.end_lineno.is_none());
        assert!(frame.source_line.is_none());
        assert!(frame.locals.is_none());
    }

    #[rstest]
    fn stack_frame_json_round_trip() {
        let mut locals = BTreeMap::new();
        locals.insert("x".into(), "42".into());

        let frame = StackFrame {
            filename: "example.py".into(),
            lineno: 10,
            end_lineno: Some(12),
            colno: Some(4),
            end_colno: Some(20),
            function: "process".into(),
            source_line: Some("    result = compute(x)".into()),
            locals: Some(locals),
        };

        let json = serde_json::to_string(&frame).expect("serialize frame");
        let decoded: StackFrame = serde_json::from_str(&json).expect("deserialize frame");
        assert_eq!(frame, decoded);
    }

    #[rstest]
    fn stack_frame_skips_none_fields_in_json() {
        let frame = StackFrame::new("test.py", 1, "main");
        let json = serde_json::to_string(&frame).expect("serialize frame");
        assert!(!json.contains("end_lineno"));
        assert!(!json.contains("source_line"));
        assert!(!json.contains("locals"));
    }

    #[rstest]
    fn stack_trace_payload_new_sets_version() {
        let payload = StackTracePayload::new(vec![]);
        assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
    }

    #[rstest]
    fn stack_trace_payload_json_round_trip() {
        let frames = vec![
            StackFrame::new("a.py", 1, "outer"),
            StackFrame::new("b.py", 2, "inner"),
        ];
        let payload = StackTracePayload::new(frames);

        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: StackTracePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[rstest]
    fn exception_payload_new_sets_version_and_message() {
        let payload = ExceptionPayload::new("KeyError", "missing key");
        assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
        assert_eq!(payload.type_name, "KeyError");
        assert_eq!(payload.message, "missing key");
        assert!(payload.cause.is_none());
        assert!(payload.context.is_none());
    }

    #[rstest]
    fn exception_payload_with_cause_chains() {
        let root = ExceptionPayload::new("IOError", "read failed");
        let wrapped = ExceptionPayload::new("RuntimeError", "operation failed").with_cause(root);

        assert!(wrapped.cause.is_some());
        let cause = wrapped.cause.as_ref().expect("cause exists");
        assert_eq!(cause.type_name, "IOError");
    }

    #[rstest]
    fn exception_payload_with_context() {
        let ctx = ExceptionPayload::new("ValueError", "bad input");
        let error = ExceptionPayload::new("TypeError", "wrong type").with_context(ctx);

        assert!(error.context.is_some());
        assert!(!error.suppress_context);
    }

    #[rstest]
    fn exception_payload_json_round_trip() {
        let frame = StackFrame::new("main.py", 100, "run");
        let cause = ExceptionPayload::new("OSError", "file not found");
        let payload = ExceptionPayload {
            schema_version: EXCEPTION_SCHEMA_VERSION,
            type_name: "RuntimeError".into(),
            module: Some("myapp.errors".into()),
            message: "failed to process".into(),
            args_repr: vec!["'path'".into(), "42".into()],
            notes: vec!["Check file permissions".into()],
            frames: vec![frame],
            cause: Some(Box::new(cause)),
            context: None,
            suppress_context: true,
            exceptions: vec![],
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[rstest]
    fn exception_payload_skips_default_fields() {
        let payload = ExceptionPayload::new("Error", "msg");
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(!json.contains("args_repr"));
        assert!(!json.contains("notes"));
        assert!(!json.contains("frames"));
        assert!(!json.contains("exceptions"));
        assert!(!json.contains("suppress_context"));
    }

    #[rstest]
    fn exception_payload_includes_suppress_context_when_true() {
        let payload = ExceptionPayload {
            suppress_context: true,
            ..ExceptionPayload::new("Error", "msg")
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("suppress_context"));
    }

    #[rstest]
    fn exception_payload_msgpack_round_trip() {
        let payload = ExceptionPayload::new("ValueError", "test")
            .with_frames(vec![StackFrame::new("test.py", 1, "main")]);

        // Use with_struct_map() for compatibility with deserialization
        let mut buf = Vec::new();
        payload
            .serialize(&mut Serializer::new(&mut buf).with_struct_map())
            .expect("serialize msgpack");
        let decoded: ExceptionPayload = rmp_serde::from_slice(&buf).expect("deserialize msgpack");
        assert_eq!(payload, decoded);
    }

    #[rstest]
    fn exception_group_with_nested_exceptions() {
        let exc1 = ExceptionPayload::new("ValueError", "bad value 1");
        let exc2 = ExceptionPayload::new("TypeError", "wrong type");

        let group = ExceptionPayload {
            schema_version: EXCEPTION_SCHEMA_VERSION,
            type_name: "ExceptionGroup".into(),
            message: "multiple errors".into(),
            exceptions: vec![exc1, exc2],
            ..Default::default()
        };

        let json = serde_json::to_string(&group).expect("serialize");
        let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.exceptions.len(), 2);
    }

    #[rstest]
    fn deep_cause_chain_serializes() {
        // Test a chain of 10 nested causes to ensure no stack overflow
        let mut current = ExceptionPayload::new("BaseError", "root cause");
        for i in 1..10 {
            current = ExceptionPayload::new(format!("Error{i}"), format!("level {i}"))
                .with_cause(current);
        }

        let json = serde_json::to_string(&current).expect("serialize deep chain");
        let decoded: ExceptionPayload = serde_json::from_str(&json).expect("deserialize");

        // Verify chain depth
        let mut depth = 0;
        let mut node = Some(&decoded);
        while let Some(n) = node {
            depth += 1;
            node = n.cause.as_deref();
        }
        assert_eq!(depth, 10);
    }

    #[rstest]
    fn types_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<StackFrame>();
        assert_send_sync::<StackTracePayload>();
        assert_send_sync::<ExceptionPayload>();
    }
}
