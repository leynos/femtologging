//! Module-level logging convenience functions for Python callers.
//!
//! Provides `debug`, `info`, `warn`, and `error` functions that mirror
//! Python's `logging.debug()`, `logging.info()`, etc. Each function uses the
//! root logger by default and captures the Python caller's source location
//! (filename, line number, module name) into the log record's metadata.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyInt};
use pyo3::{PyAny, exceptions::PyTypeError, exceptions::PyValueError};

use crate::level::FemtoLevel;
use crate::log_context;
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

fn extract_context_dict(
    context: &Bound<'_, PyAny>,
) -> PyResult<std::collections::BTreeMap<String, String>> {
    let dict = context.cast::<PyDict>().map_err(|_| {
        PyTypeError::new_err("context must be a dict[str, str|int|float|bool|None]")
    })?;
    let mut result = std::collections::BTreeMap::new();
    for (raw_key, raw_value) in dict.iter() {
        let key = raw_key
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err("context keys must be strings"))?;
        let value = extract_context_value(&raw_value)?;
        result.insert(key, value);
    }
    Ok(result)
}

fn extract_context_value(raw_value: &Bound<'_, PyAny>) -> PyResult<String> {
    if raw_value.is_none() {
        return Ok(String::from("None"));
    }
    if raw_value.extract::<bool>().is_ok() {
        return Ok(raw_value.str()?.to_str()?.to_owned());
    }
    if raw_value.is_instance_of::<PyInt>() {
        return Ok(raw_value.str()?.to_str()?.to_owned());
    }
    if raw_value.extract::<f64>().is_ok() {
        return Ok(raw_value.str()?.to_str()?.to_owned());
    }
    if let Ok(v) = raw_value.extract::<String>() {
        return Ok(v);
    }
    Err(PyTypeError::new_err(
        "context values must be str, int, float, bool, or None",
    ))
}

#[path = "convenience_functions_python_bindings.rs"]
mod python_bindings;
pub(crate) use python_bindings::{
    py_debug, py_error, py_info, py_pop_log_context, py_push_log_context, py_warn,
};

#[cfg(test)]
#[path = "convenience_functions_tests.rs"]
mod tests;
