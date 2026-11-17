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
//! # Versioning Policy
//!
//! Exception payloads include a `schema_version` field to enable schema
//! evolution without breaking consumers.
//!
//! ## Compatibility Guarantees
//!
//! - **Backward compatible**: Code supporting version N can read payloads from
//!   versions [`MIN_EXCEPTION_SCHEMA_VERSION`] through N. Missing optional
//!   fields use serde default values.
//! - **Forward incompatible**: Code supporting version N rejects payloads with
//!   version > N. Use [`validate_schema_version`] for a single value or the
//!   `validate_version` method on a payload type to check before processing.
//!
//! Validation is recursive. `cause`, `context`, and exception-group members
//! are independently versioned [`ExceptionPayload`] values, so validating a
//! payload checks the root and every payload nested beneath it. The walk is
//! depth-first in pre-order (node, `cause`, `context`, then exception-group
//! members by index), and the first unsupported version is reported with a
//! dot-separated path such as `cause.exceptions[3].context`. Traversal is
//! bounded by [`MAX_EXCEPTION_PAYLOAD_DEPTH`] and
//! [`MAX_EXCEPTION_PAYLOAD_NODES`]; those limits bound validation work, not
//! parsing.
//!
//! ## Version Increment Rules
//!
//! Increment [`EXCEPTION_SCHEMA_VERSION`] when making breaking changes:
//!
//! - Adding required fields
//! - Removing fields (even optional ones)
//! - Changing field types or semantics
//! - Renaming fields
//!
//! Non-breaking changes (no version bump required):
//!
//! - Adding optional fields with `#[serde(default)]`
//!
//! ## Validation Example
//!
//! ```rust
//! use _femtologging_rs::exception_schema::{
//!     ExceptionPayload, SchemaVersionError, SchemaVersioned,
//! };
//!
//! fn process_payload(json: &str) -> Result<(), Box<dyn std::error::Error>> {
//!     let payload: ExceptionPayload = serde_json::from_str(json)?;
//!     // Checks the root payload and every nested payload it reaches.
//!     payload.validate_version()?;
//!     // Process the validated payload...
//!     Ok(())
//! }
//! ```
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

mod filtering;
mod version_validation;
mod versioning;

pub use versioning::{SchemaVersionError, SchemaVersioned, validate_schema_version};

#[cfg(test)]
mod tests;

/// Minimum supported schema version for exception payloads.
///
/// Payloads with versions below this value are rejected during validation.
pub const MIN_EXCEPTION_SCHEMA_VERSION: u16 = 1;

/// Current schema version for exception payloads.
///
/// Increment this when making breaking changes to the schema structure.
/// Consumers should check this value when deserializing to handle
/// compatibility.
pub const EXCEPTION_SCHEMA_VERSION: u16 = 1;

/// Maximum nesting depth that recursive schema version validation traverses.
///
/// The root payload is at depth 0, and each `cause`, `context`, or
/// exception-group edge adds one. The limit matches the default recursion
/// limit of `serde_json`, so a payload that can be deserialized from JSON can
/// also be validated. It bounds validation work; it is not a parsing limit.
pub const MAX_EXCEPTION_PAYLOAD_DEPTH: usize = 128;

/// Maximum number of payload nodes that recursive validation visits.
///
/// The root counts as one node, as does every nested `cause`, `context`, and
/// exception-group member. It bounds validation work; it is not a parsing
/// limit.
pub const MAX_EXCEPTION_PAYLOAD_NODES: usize = 10_000;

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
}

impl SchemaVersioned for StackTracePayload {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl SchemaVersioned for ExceptionPayload {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Validate this payload and every nested payload it reaches.
    ///
    /// The walk is depth-first in pre-order: the payload itself, then its
    /// `cause`, its `context`, and its exception-group members in index
    /// order. A mismatch below the root is reported through
    /// [`SchemaVersionError::Nested`] with a path to the offending payload.
    ///
    /// # Errors
    ///
    /// Returns a plain [`SchemaVersionError::VersionTooNew`] or
    /// [`SchemaVersionError::VersionTooOld`] when the root is unsupported,
    /// [`SchemaVersionError::Nested`] when a nested payload is unsupported,
    /// [`SchemaVersionError::TreeTooDeep`] when the tree is deeper than
    /// [`MAX_EXCEPTION_PAYLOAD_DEPTH`], or
    /// [`SchemaVersionError::TreeTooLarge`] when it holds more than
    /// [`MAX_EXCEPTION_PAYLOAD_NODES`] nodes.
    fn validate_version(&self) -> Result<(), SchemaVersionError> {
        version_validation::validate_payload_tree(self)
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
}
