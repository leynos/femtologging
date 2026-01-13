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
//!   version > N. Use [`validate_schema_version`] or the `validate_version`
//!   methods on payload types to check before processing.
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
use thiserror::Error;

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

/// Errors related to exception schema version validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SchemaVersionError {
    /// The payload schema version is newer than supported.
    #[error(
        "unsupported exception schema version: found {found}, \
         maximum supported is {max_supported}"
    )]
    VersionTooNew {
        /// The schema version found in the payload.
        found: u16,
        /// The maximum schema version supported by this library.
        max_supported: u16,
    },
    /// The payload schema version is older than supported.
    #[error(
        "unsupported exception schema version: found {found}, \
         minimum supported is {min_supported}"
    )]
    VersionTooOld {
        /// The schema version found in the payload.
        found: u16,
        /// The minimum schema version supported by this library.
        min_supported: u16,
    },
}

/// Validate that a schema version is supported.
///
/// Returns `Ok(())` if the version is in the supported range
/// ([`MIN_EXCEPTION_SCHEMA_VERSION`] through [`EXCEPTION_SCHEMA_VERSION`]),
/// or an error if the version is unsupported.
///
/// # Errors
///
/// Returns [`SchemaVersionError::VersionTooNew`] if the version exceeds
/// [`EXCEPTION_SCHEMA_VERSION`], or [`SchemaVersionError::VersionTooOld`]
/// if the version is below [`MIN_EXCEPTION_SCHEMA_VERSION`].
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::{
///     validate_schema_version, EXCEPTION_SCHEMA_VERSION, MIN_EXCEPTION_SCHEMA_VERSION,
/// };
///
/// assert!(validate_schema_version(MIN_EXCEPTION_SCHEMA_VERSION).is_ok());
/// assert!(validate_schema_version(EXCEPTION_SCHEMA_VERSION).is_ok());
/// assert!(validate_schema_version(EXCEPTION_SCHEMA_VERSION + 1).is_err());
/// ```
pub fn validate_schema_version(version: u16) -> Result<(), SchemaVersionError> {
    if version > EXCEPTION_SCHEMA_VERSION {
        return Err(SchemaVersionError::VersionTooNew {
            found: version,
            max_supported: EXCEPTION_SCHEMA_VERSION,
        });
    }
    if version < MIN_EXCEPTION_SCHEMA_VERSION {
        return Err(SchemaVersionError::VersionTooOld {
            found: version,
            min_supported: MIN_EXCEPTION_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Trait for types that carry a schema version.
///
/// Implementing this trait provides a blanket `validate_version` method
/// via the extension trait pattern.
pub trait SchemaVersioned {
    /// Returns the schema version of this payload.
    fn schema_version(&self) -> u16;

    /// Validate that this payload's schema version is supported.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaVersionError::VersionTooNew`] if the version exceeds
    /// the maximum supported, or [`SchemaVersionError::VersionTooOld`] if
    /// below the minimum supported.
    fn validate_version(&self) -> Result<(), SchemaVersionError> {
        validate_schema_version(self.schema_version())
    }
}

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

impl SchemaVersioned for ExceptionPayload {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

#[cfg(test)]
#[path = "exception_schema_tests.rs"]
mod tests;
