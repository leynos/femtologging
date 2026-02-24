//! Convenience logging methods for stdlib-style usage.
//!
//! This module adds `isEnabledFor`, `debug`, `info`, `warning`, `error`,
//! `critical`, and `exception` to [`FemtoLogger`] via a separate
//! `#[pymethods]` impl block, keeping the main `mod.rs` within the
//! repository's 400-line file limit.
//!
//! Unlike the stdlib, these methods accept a pre-formatted `message`
//! string rather than `*args` / `**kwargs` lazy formatting.

use pyo3::PyAny;
use pyo3::prelude::*;
use pyo3::types::PyBool;

use crate::level::FemtoLevel;

use super::FemtoLogger;

#[pymethods]
impl FemtoLogger {
    /// Return whether a message at the given level would be processed.
    ///
    /// This method mirrors Python's ``logging.Logger.isEnabledFor()``.
    ///
    /// # Parameters
    ///
    /// - `level`: The log level to test (e.g., "INFO", "DEBUG").
    ///
    /// # Returns
    ///
    /// `True` when the logger's effective level would allow a record at
    /// the given level through.
    ///
    /// # Examples
    ///
    /// ```python
    /// logger = FemtoLogger("app")
    /// logger.set_level("WARNING")
    /// assert not logger.isEnabledFor("DEBUG")
    /// assert logger.isEnabledFor("ERROR")
    /// ```
    #[pyo3(name = "isEnabledFor", text_signature = "(self, level)")]
    pub fn py_is_enabled_for(&self, level: FemtoLevel) -> bool {
        self.is_enabled_for(level)
    }

    /// Log a message at DEBUG level.
    ///
    /// Delegates to the internal logging machinery with a fixed level.
    ///
    /// # Examples
    ///
    /// ```python
    /// logger.debug(f"cache hit for {key}")
    /// ```
    #[pyo3(
        name = "debug",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_debug(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        self.py_log(py, FemtoLevel::Debug, message, exc_info, stack_info)
    }

    /// Log a message at INFO level.
    ///
    /// Delegates to the internal logging machinery with a fixed level.
    ///
    /// # Examples
    ///
    /// ```python
    /// logger.info(f"server started on port {port}")
    /// ```
    #[pyo3(
        name = "info",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_info(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        self.py_log(py, FemtoLevel::Info, message, exc_info, stack_info)
    }

    /// Log a message at WARN level.
    ///
    /// Delegates to the internal logging machinery with a fixed level.
    ///
    /// # Examples
    ///
    /// ```python
    /// logger.warning("disk usage above 90%")
    /// ```
    #[pyo3(
        name = "warning",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_warning(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        self.py_log(py, FemtoLevel::Warn, message, exc_info, stack_info)
    }

    /// Log a message at ERROR level.
    ///
    /// Delegates to the internal logging machinery with a fixed level.
    ///
    /// # Examples
    ///
    /// ```python
    /// logger.error("connection refused")
    /// ```
    #[pyo3(
        name = "error",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_error(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        self.py_log(py, FemtoLevel::Error, message, exc_info, stack_info)
    }

    /// Log a message at CRITICAL level.
    ///
    /// Delegates to the internal logging machinery with a fixed level.
    ///
    /// # Examples
    ///
    /// ```python
    /// logger.critical("out of memory, shutting down")
    /// ```
    #[pyo3(
        name = "critical",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_critical(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        self.py_log(py, FemtoLevel::Critical, message, exc_info, stack_info)
    }

    /// Log a message at ERROR level with ``exc_info`` defaulting to ``True``.
    ///
    /// This mirrors Python's ``logging.Logger.exception()`` which behaves
    /// like ``error()`` but automatically captures the active exception.
    ///
    /// # Examples
    ///
    /// ```python
    /// try:
    ///     risky_call()
    /// except Exception:
    ///     logger.exception("risky_call failed")
    /// ```
    #[pyo3(
        name = "exception",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_exception(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        let effective_exc_info = exc_info.map_or_else(
            || PyBool::new(py, true).to_owned().into_any(),
            |v| v.to_owned(),
        );
        self.py_log(
            py,
            FemtoLevel::Error,
            message,
            Some(&effective_exc_info.as_borrowed()),
            stack_info,
        )
    }
}
