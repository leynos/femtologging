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
                        signature = (message, /, *, exc_info=None, stack_info=false),
                        text_signature = "(self, message, /, *, exc_info=None, stack_info=False)"
                    )]
            pub fn $fn_name(
                &self,
                py: Python<'_>,
                message: &str,
                exc_info: Option<&Bound<'_, PyAny>>,
                stack_info: Option<bool>,
            ) -> PyResult<Option<String>> {
                self.py_log(py, $level, message, exc_info, stack_info)
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
                let py_true = PyBool::new(py, true);
                self.py_log(
                    py,
                    FemtoLevel::Error,
                    message,
                    Some(py_true.as_any()),
                    stack_info,
                )
            }
            Some(val) => self.py_log(py, FemtoLevel::Error, message, Some(val), stack_info),
        }
    }
}
