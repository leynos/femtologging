//! Frame filtering methods for `StackTracePayload` and `ExceptionPayload`.
//!
//! This module contains extension methods for filtering stack frames
//! from payloads using various criteria.

use super::{ExceptionPayload, StackFrame, StackTracePayload};
use crate::frame_filter;

impl StackTracePayload {
    /// Private helper to apply a frame transformation and return a new payload.
    fn apply_frame_transform<F>(&self, transform: F) -> Self
    where
        F: FnOnce(&[StackFrame]) -> Vec<StackFrame>,
    {
        Self {
            schema_version: self.schema_version,
            frames: transform(&self.frames),
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
        self.apply_frame_transform(|frames| {
            frames.iter().filter(|f| predicate(f)).cloned().collect()
        })
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
        self.apply_frame_transform(|frames| frame_filter::limit_frames(frames, n))
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
        self.apply_frame_transform(|frames| frame_filter::exclude_by_filename(frames, patterns))
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
        self.apply_frame_transform(|frames| frame_filter::exclude_by_function(frames, patterns))
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
        self.apply_frame_transform(frame_filter::exclude_logging_infrastructure)
    }
}

impl ExceptionPayload {
    /// Private helper to apply a frame transformation recursively across
    /// the exception payload, its cause chain, context chain, and exception groups.
    fn apply_frame_transform<F>(&self, transform: &F) -> Self
    where
        F: Fn(&[StackFrame]) -> Vec<StackFrame>,
    {
        Self {
            schema_version: self.schema_version,
            type_name: self.type_name.clone(),
            module: self.module.clone(),
            message: self.message.clone(),
            args_repr: self.args_repr.clone(),
            notes: self.notes.clone(),
            frames: transform(&self.frames),
            cause: self
                .cause
                .as_ref()
                .map(|c| Box::new(c.apply_frame_transform(transform))),
            context: self
                .context
                .as_ref()
                .map(|c| Box::new(c.apply_frame_transform(transform))),
            suppress_context: self.suppress_context,
            exceptions: self
                .exceptions
                .iter()
                .map(|e| e.apply_frame_transform(transform))
                .collect(),
        }
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
        F: Fn(&StackFrame) -> bool,
    {
        self.apply_frame_transform(&|frames| {
            frames.iter().filter(|f| predicate(f)).cloned().collect()
        })
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
        self.apply_frame_transform(&|frames| frame_filter::limit_frames(frames, n))
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
        self.apply_frame_transform(&|frames| frame_filter::exclude_by_filename(frames, patterns))
    }

    /// Return a new payload excluding frames matching function name patterns.
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
    ///     StackFrame::new("app.py", 10, "main"),
    ///     StackFrame::new("app.py", 20, "_internal_helper"),
    /// ];
    /// let payload = ExceptionPayload::new("Error", "test").with_frames(frames);
    ///
    /// let filtered = payload.exclude_functions(&["_internal"]);
    /// assert_eq!(filtered.frames.len(), 1);
    /// ```
    #[must_use]
    pub fn exclude_functions(&self, patterns: &[&str]) -> Self {
        self.apply_frame_transform(&|frames| frame_filter::exclude_by_function(frames, patterns))
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
        self.apply_frame_transform(&frame_filter::exclude_logging_infrastructure)
    }
}
