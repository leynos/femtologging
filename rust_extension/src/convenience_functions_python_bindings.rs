//! `PyO3` convenience-function wrappers isolated from logging helpers.

use super::*;

/// Push a structured logging context frame for the current thread.
#[pyfunction(name = "_push_log_context", signature = (context), text_signature = "(context)")]
pub(crate) fn py_push_log_context(context: &Bound<'_, PyAny>) -> PyResult<()> {
    let context_map = extract_context_dict(context)?;
    log_context::push_log_context_map(context_map)
        .map_err(|err| PyValueError::new_err(err.to_string()))
}

/// Pop the latest structured logging context frame for the current thread.
#[pyfunction(name = "_pop_log_context", text_signature = "()")]
pub(crate) fn py_pop_log_context() -> PyResult<()> {
    log_context::pop_log_context().map_err(|err| PyValueError::new_err(err.to_string()))
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
