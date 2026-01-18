//! Frame filtering utilities for stack trace and exception payloads.
//!
//! This module provides functions for filtering stack frames from exception
//! and stack trace payloads. Common use cases include removing logging
//! infrastructure frames, limiting stack depth, and excluding frames by
//! filename or function name patterns.
//!
//! # Usage
//!
//! The primary entry points are:
//! - [`filter_frames`] - Filter frames using a predicate function
//! - [`limit_frames`] - Keep only the N most recent frames
//! - [`exclude_by_filename`] - Exclude frames matching filename patterns
//! - [`exclude_by_function`] - Exclude frames matching function patterns
//! - [`exclude_logging_infrastructure`] - Remove common logging framework frames
//!
//! # Example
//!
//! ```rust
//! use _femtologging_rs::exception_schema::{StackFrame, StackTracePayload};
//! use _femtologging_rs::frame_filter::{exclude_logging_infrastructure, limit_frames};
//!
//! let frames = vec![
//!     StackFrame::new("app.py", 10, "main"),
//!     StackFrame::new("femtologging/__init__.py", 20, "info"),
//!     StackFrame::new("logging/__init__.py", 30, "_log"),
//! ];
//! let payload = StackTracePayload::new(frames);
//!
//! // Remove logging infrastructure frames
//! let filtered = exclude_logging_infrastructure(&payload.frames);
//! assert_eq!(filtered.len(), 1);
//! assert_eq!(filtered[0].filename, "app.py");
//! ```

use crate::exception_schema::StackFrame;

#[cfg(test)]
mod tests;

/// Filename patterns that identify logging infrastructure frames.
///
/// These patterns match common logging framework paths that are typically
/// not useful in application stack traces.
pub const LOGGING_INFRA_PATTERNS: &[&str] = &[
    "femtologging",
    "_femtologging_rs",
    "logging/__init__",
    "logging/config",
    "logging/handlers",
    "<frozen importlib",
];

/// Filter frames using a predicate function.
///
/// Returns a new vector containing only frames for which the predicate
/// returns `true`.
///
/// # Parameters
///
/// * `frames` - The frames to filter.
/// * `predicate` - A function that returns `true` for frames to keep.
///
/// # Returns
///
/// A new vector of frames that satisfy the predicate.
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::StackFrame;
/// use _femtologging_rs::frame_filter::filter_frames;
///
/// let frames = vec![
///     StackFrame::new("app.py", 10, "main"),
///     StackFrame::new("lib.py", 20, "helper"),
/// ];
///
/// // Keep only frames from app.py
/// let filtered = filter_frames(&frames, |f| f.filename == "app.py");
/// assert_eq!(filtered.len(), 1);
/// ```
pub fn filter_frames<F>(frames: &[StackFrame], predicate: F) -> Vec<StackFrame>
where
    F: Fn(&StackFrame) -> bool,
{
    frames.iter().filter(|f| predicate(f)).cloned().collect()
}

/// Limit frames to the N most recent (last N in the list).
///
/// Stack frames are typically ordered from oldest to newest (outermost to
/// innermost call). This function keeps the last `n` frames, which are the
/// most recent calls closest to where the exception occurred.
///
/// # Parameters
///
/// * `frames` - The frames to limit.
/// * `n` - Maximum number of frames to keep.
///
/// # Returns
///
/// A new vector with at most `n` frames from the end of the input.
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::StackFrame;
/// use _femtologging_rs::frame_filter::limit_frames;
///
/// let frames = vec![
///     StackFrame::new("a.py", 1, "outer"),
///     StackFrame::new("b.py", 2, "middle"),
///     StackFrame::new("c.py", 3, "inner"),
/// ];
///
/// let limited = limit_frames(&frames, 2);
/// assert_eq!(limited.len(), 2);
/// assert_eq!(limited[0].filename, "b.py");
/// assert_eq!(limited[1].filename, "c.py");
/// ```
pub fn limit_frames(frames: &[StackFrame], n: usize) -> Vec<StackFrame> {
    if frames.len() <= n {
        return frames.to_vec();
    }
    frames[frames.len() - n..].to_vec()
}

/// Check if a filename matches any of the given patterns.
///
/// Patterns are matched as substrings of the filename.
fn matches_any_pattern(filename: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| filename.contains(p))
}

/// Exclude frames whose filename matches any of the given patterns.
///
/// Patterns are matched as substrings of the filename. For example, the
/// pattern `"logging"` would match `"/usr/lib/python3.11/logging/__init__.py"`.
///
/// # Parameters
///
/// * `frames` - The frames to filter.
/// * `patterns` - Filename patterns to exclude (substring matching).
///
/// # Returns
///
/// A new vector excluding frames that match any pattern.
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::StackFrame;
/// use _femtologging_rs::frame_filter::exclude_by_filename;
///
/// let frames = vec![
///     StackFrame::new("myapp/main.py", 10, "main"),
///     StackFrame::new(".venv/lib/requests/api.py", 20, "get"),
///     StackFrame::new("myapp/utils.py", 30, "helper"),
/// ];
///
/// let filtered = exclude_by_filename(&frames, &[".venv/"]);
/// assert_eq!(filtered.len(), 2);
/// ```
pub fn exclude_by_filename(frames: &[StackFrame], patterns: &[&str]) -> Vec<StackFrame> {
    filter_frames(frames, |f| !matches_any_pattern(&f.filename, patterns))
}

/// Exclude frames whose function name matches any of the given patterns.
///
/// Patterns are matched as substrings of the function name.
///
/// # Parameters
///
/// * `frames` - The frames to filter.
/// * `patterns` - Function name patterns to exclude (substring matching).
///
/// # Returns
///
/// A new vector excluding frames that match any pattern.
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::StackFrame;
/// use _femtologging_rs::frame_filter::exclude_by_function;
///
/// let frames = vec![
///     StackFrame::new("app.py", 10, "main"),
///     StackFrame::new("app.py", 20, "_internal_helper"),
///     StackFrame::new("app.py", 30, "public_api"),
/// ];
///
/// // Exclude internal functions
/// let filtered = exclude_by_function(&frames, &["_internal"]);
/// assert_eq!(filtered.len(), 2);
/// ```
pub fn exclude_by_function(frames: &[StackFrame], patterns: &[&str]) -> Vec<StackFrame> {
    filter_frames(frames, |f| !matches_any_pattern(&f.function, patterns))
}

/// Exclude frames from common logging infrastructure.
///
/// This removes frames from femtologging, Python's standard logging module,
/// and import machinery that are typically not useful in application traces.
///
/// Uses the patterns defined in [`LOGGING_INFRA_PATTERNS`].
///
/// # Parameters
///
/// * `frames` - The frames to filter.
///
/// # Returns
///
/// A new vector excluding logging infrastructure frames.
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::StackFrame;
/// use _femtologging_rs::frame_filter::exclude_logging_infrastructure;
///
/// let frames = vec![
///     StackFrame::new("myapp/main.py", 10, "run"),
///     StackFrame::new("femtologging/__init__.py", 50, "info"),
///     StackFrame::new("logging/__init__.py", 100, "_log"),
/// ];
///
/// let filtered = exclude_logging_infrastructure(&frames);
/// assert_eq!(filtered.len(), 1);
/// assert_eq!(filtered[0].filename, "myapp/main.py");
/// ```
pub fn exclude_logging_infrastructure(frames: &[StackFrame]) -> Vec<StackFrame> {
    exclude_by_filename(frames, LOGGING_INFRA_PATTERNS)
}

/// Check if a frame is from logging infrastructure.
///
/// Returns `true` if the frame's filename matches any of the patterns in
/// [`LOGGING_INFRA_PATTERNS`].
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::exception_schema::StackFrame;
/// use _femtologging_rs::frame_filter::is_logging_infrastructure;
///
/// let app_frame = StackFrame::new("myapp/main.py", 10, "run");
/// let log_frame = StackFrame::new("logging/__init__.py", 50, "info");
///
/// assert!(!is_logging_infrastructure(&app_frame));
/// assert!(is_logging_infrastructure(&log_frame));
/// ```
pub fn is_logging_infrastructure(frame: &StackFrame) -> bool {
    matches_any_pattern(&frame.filename, LOGGING_INFRA_PATTERNS)
}
