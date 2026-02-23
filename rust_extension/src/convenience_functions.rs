//! Module-level logging convenience functions for Python callers.
//!
//! Provides `debug`, `info`, `warn`, and `error` functions that mirror
//! Python's `logging.debug()`, `logging.info()`, etc. Each function uses the
//! root logger by default and captures the Python caller's source location
//! (filename, line number, module name) into the log record's metadata.

use pyo3::prelude::*;

use crate::level::FemtoLevel;
use crate::log_record::RecordMetadata;
use crate::manager;

/// Default logger name used when the caller does not specify one.
const DEFAULT_LOGGER_NAME: &str = "root";

/// Extract the Python caller's source location from the call stack.
///
/// Calls `sys._getframe(depth)` to retrieve the caller's frame, then
/// reads `f_code.co_filename`, `f_lineno`, and `f_globals['__name__']`.
/// Falls back to empty strings and zero line number if frame
/// inspection fails (e.g., on non-CPython interpreters).
fn capture_python_caller(py: Python<'_>, depth: i32) -> RecordMetadata {
    let (filename, lineno, module_name) =
        extract_frame_info(py, depth).unwrap_or_else(|_| (String::new(), 0, String::new()));

    RecordMetadata {
        module_path: module_name,
        filename,
        line_number: lineno,
        ..Default::default()
    }
}

/// Attempt to extract frame info from the Python call stack.
///
/// Returns a tuple of `(filename, line_number, module_name)`.
/// The module name is read from `frame.f_globals['__name__']`; if the
/// key is absent (e.g., in embedded contexts) it falls back to an
/// empty string rather than aborting the entire extraction.
fn extract_frame_info(py: Python<'_>, depth: i32) -> PyResult<(String, u32, String)> {
    let sys = py.import("sys")?;
    let frame = sys.call_method1("_getframe", (depth,))?;
    let code = frame.getattr("f_code")?;
    let filename: String = code.getattr("co_filename")?.extract()?;
    let lineno: u32 = frame.getattr("f_lineno")?.extract()?;
    let module_name: String = frame
        .getattr("f_globals")?
        .get_item("__name__")
        .and_then(|v| v.extract())
        .unwrap_or_default();
    Ok((filename, lineno, module_name))
}

/// Shared implementation for all convenience logging functions.
///
/// Resolves the target logger, captures the caller's source location,
/// and dispatches a log record at the specified level.
fn log_at_level(
    py: Python<'_>,
    level: FemtoLevel,
    message: &str,
    name: Option<&str>,
) -> PyResult<Option<String>> {
    let logger_name = name.unwrap_or(DEFAULT_LOGGER_NAME);
    let logger = manager::get_logger(py, logger_name)?;
    // Rust functions are transparent in the Python frame stack, so
    // _getframe(0) = the pyfunction, _getframe(1) = the Python caller.
    let metadata = capture_python_caller(py, 1);
    Ok(logger
        .borrow(py)
        .log_with_metadata(level, message, metadata))
}

/// Log a message at DEBUG level.
///
/// Uses the root logger by default. Pass `name` to target a specific logger.
///
/// Parameters
/// ----------
/// message : str
///     The log message.
/// name : str, optional
///     Logger name. Defaults to "root".
///
/// Returns
/// -------
/// str or None
///     The formatted message if the record passed level and filter checks,
///     otherwise None.
///
/// Examples
/// --------
/// ```python
/// import femtologging
/// femtologging.debug("entering request handler")
/// femtologging.debug("query executed", name="db")
/// ```
#[pyfunction]
#[pyo3(
    name = "debug",
    signature = (message, /, *, name=None),
    text_signature = "(message, /, *, name=None)"
)]
pub(crate) fn py_debug(
    py: Python<'_>,
    message: &str,
    name: Option<&str>,
) -> PyResult<Option<String>> {
    log_at_level(py, FemtoLevel::Debug, message, name)
}

/// Log a message at INFO level.
///
/// Uses the root logger by default. Pass `name` to target a specific logger.
///
/// Parameters
/// ----------
/// message : str
///     The log message.
/// name : str, optional
///     Logger name. Defaults to "root".
///
/// Returns
/// -------
/// str or None
///     The formatted message if the record passed level and filter checks,
///     otherwise None.
///
/// Examples
/// --------
/// ```python
/// import femtologging
/// femtologging.info("server started on port 8080")
/// ```
#[pyfunction]
#[pyo3(
    name = "info",
    signature = (message, /, *, name=None),
    text_signature = "(message, /, *, name=None)"
)]
pub(crate) fn py_info(
    py: Python<'_>,
    message: &str,
    name: Option<&str>,
) -> PyResult<Option<String>> {
    log_at_level(py, FemtoLevel::Info, message, name)
}

/// Log a message at WARN level.
///
/// Uses the root logger by default. Pass `name` to target a specific logger.
///
/// Parameters
/// ----------
/// message : str
///     The log message.
/// name : str, optional
///     Logger name. Defaults to "root".
///
/// Returns
/// -------
/// str or None
///     The formatted message if the record passed level and filter checks,
///     otherwise None.
///
/// Examples
/// --------
/// ```python
/// import femtologging
/// femtologging.warn("disk space running low")
/// ```
#[pyfunction]
#[pyo3(
    name = "warn",
    signature = (message, /, *, name=None),
    text_signature = "(message, /, *, name=None)"
)]
pub(crate) fn py_warn(
    py: Python<'_>,
    message: &str,
    name: Option<&str>,
) -> PyResult<Option<String>> {
    log_at_level(py, FemtoLevel::Warn, message, name)
}

/// Log a message at ERROR level.
///
/// Uses the root logger by default. Pass `name` to target a specific logger.
///
/// Parameters
/// ----------
/// message : str
///     The log message.
/// name : str, optional
///     Logger name. Defaults to "root".
///
/// Returns
/// -------
/// str or None
///     The formatted message if the record passed level and filter checks,
///     otherwise None.
///
/// Examples
/// --------
/// ```python
/// import femtologging
/// femtologging.error("connection to database lost")
/// ```
#[pyfunction]
#[pyo3(
    name = "error",
    signature = (message, /, *, name=None),
    text_signature = "(message, /, *, name=None)"
)]
pub(crate) fn py_error(
    py: Python<'_>,
    message: &str,
    name: Option<&str>,
) -> PyResult<Option<String>> {
    log_at_level(py, FemtoLevel::Error, message, name)
}

#[cfg(test)]
#[path = "convenience_functions_tests.rs"]
mod tests;
