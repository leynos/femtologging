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
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBool;
#[cfg(not(feature = "python"))]
use std::collections::BTreeMap;

use crate::level::FemtoLevel;
use crate::log_context;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
#[cfg(feature = "python")]
use crate::python_context::extract_python_context_map;
#[cfg(feature = "python")]
use crate::traceback_capture;

use super::FemtoLogger;
#[cfg(feature = "python")]
use super::python_helpers::capture_exception_payload;

#[pymethods]
impl FemtoLogger {
    /// Emit a message at the provided level and return its formatted text.
    ///
    /// The record includes active scoped context merged with optional inline
    /// fields. Inline fields override scoped fields with the same key. Context
    /// is validated before the logger's level gate, so invalid fields raise an
    /// error even when the message would not otherwise be emitted.
    ///
    /// # Parameters
    ///
    /// - `level`: The level at which to emit the message.
    /// - `message`: The already-formatted message text.
    /// - `exc_info`: Optional exception information to attach to the record.
    /// - `stack_info`: Whether to attach the current Python stack.
    /// - `extra`: Optional mapping of string keys to `str`, `int`, `float`,
    ///   `bool`, or `None` values. Context is limited to 64 unique keys, 64
    ///   UTF-8 bytes per key, 1,024 UTF-8 bytes per value, and 16 KiB total
    ///   retained key/value bytes. Unsupported mappings or values raise
    ///   `TypeError`; values exceeding these limits raise `ValueError`.
    ///
    /// # Returns
    ///
    /// The formatted message when the level is enabled, or `None` when it is
    /// disabled.
    #[pyo3(
        name = "log",
        signature = (level, message, /, *, exc_info=None, stack_info=false, extra=None),
        text_signature = "(self, level, message, /, *, exc_info=None, stack_info=False, extra=None)"
    )]
    #[cfg_attr(
        not(feature = "python"),
        expect(
            unused_variables,
            reason = "py parameter is only used when python feature is enabled"
        )
    )]
    #[cfg_attr(
        not(feature = "python"),
        expect(
            unused_mut,
            reason = "record is only mutated when python feature is enabled"
        )
    )]
    pub fn py_log(
        &self,
        py: Python<'_>,
        level: FemtoLevel,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
        extra: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<String>> {
        #[cfg(feature = "python")]
        let explicit_key_values = extra
            .map(extract_python_context_map)
            .transpose()?
            .unwrap_or_default();
        #[cfg(not(feature = "python"))]
        let explicit_key_values = BTreeMap::new();
        let merged_key_values = log_context::merge_context_values(&explicit_key_values)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        if !self.is_enabled_for(level) {
            return Ok(None);
        }
        let mut record = FemtoLogRecord::with_metadata(
            &self.name,
            level,
            message,
            RecordMetadata {
                key_values: merged_key_values,
                ..Default::default()
            },
        );

        #[cfg(feature = "python")]
        if let Some(payload) = capture_exception_payload(py, exc_info)? {
            record.set_exception_payload(payload);
        }

        #[cfg(feature = "python")]
        if stack_info.unwrap_or(false) {
            record.set_stack_payload(traceback_capture::capture_stack(py)?);
        }

        Ok(self.log_record(record))
    }
}

/// Generate a convenience logging method that delegates to `py_log` with a
/// fixed level.
///
/// PyO3 does not allow macro invocations inside `#[pymethods]` blocks, so
/// each call emits its own block (the `multiple-pymethods` Cargo feature is
/// already enabled for exactly this reason).
macro_rules! log_method {
    ($fn_name:ident, $py_name:literal, $level:expr, $doc:expr) => {
        #[pymethods]
        impl FemtoLogger {
            #[doc = $doc]
            #[pyo3(
                        name = $py_name,
                        signature = (message, /, *, exc_info=None, stack_info=false, extra=None),
                        text_signature = "(self, message, /, *, exc_info=None, stack_info=False, extra=None)"
                    )]
            pub fn $fn_name(
                &self,
                py: Python<'_>,
                message: &str,
                exc_info: Option<&Bound<'_, PyAny>>,
                stack_info: Option<bool>,
                extra: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Option<String>> {
                self.py_log(py, $level, message, exc_info, stack_info, extra)
            }
        }
    };
}

log_method!(
    py_debug,
    "debug",
    FemtoLevel::Debug,
    concat!(
        "Log a message at DEBUG level.\n",
        "\n",
        "Delegates to the internal logging machinery with a fixed level.\n",
        "\n",
        "# Examples\n",
        "\n",
        "```python\n",
        "logger.debug(f\"cache hit for {key}\")\n",
        "```"
    )
);

log_method!(
    py_info,
    "info",
    FemtoLevel::Info,
    concat!(
        "Log a message at INFO level.\n",
        "\n",
        "Delegates to the internal logging machinery with a fixed level.\n",
        "\n",
        "# Examples\n",
        "\n",
        "```python\n",
        "logger.info(f\"server started on port {port}\")\n",
        "```"
    )
);

log_method!(
    py_warning,
    "warning",
    FemtoLevel::Warn,
    concat!(
        "Log a message at WARN level.\n",
        "\n",
        "Delegates to the internal logging machinery with a fixed level.\n",
        "\n",
        "# Examples\n",
        "\n",
        "```python\n",
        "logger.warning(\"disk usage above 90%\")\n",
        "```"
    )
);

log_method!(
    py_error,
    "error",
    FemtoLevel::Error,
    concat!(
        "Log a message at ERROR level.\n",
        "\n",
        "Delegates to the internal logging machinery with a fixed level.\n",
        "\n",
        "# Examples\n",
        "\n",
        "```python\n",
        "logger.error(\"connection refused\")\n",
        "```"
    )
);

log_method!(
    py_critical,
    "critical",
    FemtoLevel::Critical,
    concat!(
        "Log a message at CRITICAL level.\n",
        "\n",
        "Delegates to the internal logging machinery with a fixed level.\n",
        "\n",
        "# Examples\n",
        "\n",
        "```python\n",
        "logger.critical(\"out of memory, shutting down\")\n",
        "```"
    )
);

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

    /// Low-level implementation of ``exception()`` for the Python wrapper.
    ///
    /// When ``exc_info`` is omitted (Rust ``None``), the method substitutes
    /// Python ``True`` to auto-capture the active exception.  A Python-level
    /// wrapper in ``_compat.py`` uses a sentinel to distinguish an omitted
    /// ``exc_info`` from an explicit ``None``, forwarding ``exc_info=True``
    /// only when the argument was genuinely omitted.
    ///
    /// # Examples
    ///
    /// ```python
    /// # Called via the Python wrapper, not directly:
    /// logger.exception("risky_call failed")
    /// ```
    #[pyo3(
        name = "_exception_impl",
        signature = (message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, message, /, *, exc_info=True, stack_info=False)"
    )]
    pub fn py_exception_impl(
        &self,
        py: Python<'_>,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        // Omitted exc_info (Rust None) → default to Python True (auto-capture).
        // Note: PyO3 maps both omitted and explicit exc_info=None from Python to
        // Rust None, so callers should use exc_info=False to suppress capture.
        match exc_info {
            None => {
                let py_true = PyBool::new(py, true).to_owned().into_any();
                self.py_log(
                    py,
                    FemtoLevel::Error,
                    message,
                    Some(&py_true),
                    stack_info,
                    None,
                )
            }
            Some(val) => self.py_log(py, FemtoLevel::Error, message, Some(val), stack_info, None),
        }
    }
}
