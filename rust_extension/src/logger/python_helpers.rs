//! Python helper functions for [`FemtoLogger`].
//!
//! This module contains utility functions for Python integration, including
//! exception capture logic and raw Python logging-call parsing.

use std::{collections::BTreeMap, io::Write};

#[cfg(feature = "python")]
use pyo3::types::PyBool;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    pybacked::PyBackedStr,
    types::{PyDict, PyTuple},
};

use super::FemtoLogger;
#[cfg(feature = "python")]
use crate::traceback_capture;
use crate::{
    level::FemtoLevel,
    log_context,
    log_record::{FemtoLogRecord, RecordMetadata},
};

/// Parsed options shared by the Python logging entry points.
pub(super) struct PythonLogOptions<'py> {
    pub(super) message: PyBackedStr,
    pub(super) exc_info: Option<Bound<'py, PyAny>>,
    pub(super) stack_info: bool,
}

/// A parsed Python logging call with its resolved level and options.
pub(super) struct PythonLogRequest<'py> {
    pub(super) level: FemtoLevel,
    pub(super) options: PythonLogOptions<'py>,
}

/// Parse the positional-only `level` and `message` accepted by `log()`.
///
/// # Errors
///
/// Returns `TypeError` when the call does not match the Python-facing
/// signature, or an extraction error when the level or message is invalid.
pub(super) fn parse_log_call<'py>(
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<PythonLogRequest<'py>> {
    let options = parse_options(args, kwargs, 2, 1)?;
    let level = args.get_item(0)?.extract()?;
    Ok(PythonLogRequest { level, options })
}

/// Parse a fixed-level convenience method's `message` and keyword options.
///
/// # Errors
///
/// Returns `TypeError` when the call does not match the Python-facing
/// signature, or an extraction error when an option has an invalid type.
pub(super) fn parse_fixed_level_call<'py>(
    level: FemtoLevel,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<PythonLogRequest<'py>> {
    let options = parse_options(args, kwargs, 1, 0)?;
    Ok(PythonLogRequest { level, options })
}

/// Parse, build, and emit a record from a Python logging call.
///
/// # Errors
///
/// Returns parsing errors before checking the resolved level. Enabled calls
/// can additionally return exception-capture or stack-capture errors.
pub(super) fn log_python_request<'py>(
    logger: &FemtoLogger,
    py: Python<'py>,
    request_result: PyResult<PythonLogRequest<'py>>,
) -> PyResult<Option<String>> {
    let request = request_result?;
    if !logger.is_enabled_for(request.level) {
        return Ok(None);
    }
    let explicit_key_values = BTreeMap::new();
    let merged_key_values = match log_context::merge_context_values(&explicit_key_values) {
        Ok(key_values) => key_values,
        Err(err) => {
            let mut stderr = std::io::stderr().lock();
            // A stderr failure cannot be reported through this logger without
            // recursively attempting to log the same dropped record.
            drop(writeln!(
                stderr,
                "FemtoLogger: dropping record due to invalid context payload: {err}"
            ));
            return Ok(None);
        }
    };
    let log_record = FemtoLogRecord::with_metadata(
        &logger.name,
        request.level,
        request.options.message.as_str(),
        RecordMetadata {
            key_values: merged_key_values,
            ..Default::default()
        },
    );

    #[cfg(feature = "python")]
    {
        let mut enriched_record = log_record;
        if let Some(payload) = capture_exception_payload(py, request.options.exc_info.as_ref())? {
            enriched_record.set_exception_payload(payload);
        }
        if request.options.stack_info {
            enriched_record.set_stack_payload(traceback_capture::capture_stack(py)?);
        }
        Ok(logger.log_record(enriched_record))
    }

    #[cfg(not(feature = "python"))]
    {
        let _ = (
            py,
            request.options.exc_info.as_ref(),
            request.options.stack_info,
        );
        Ok(logger.log_record(log_record))
    }
}

fn parse_options<'py>(
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
    expected_positional_count: usize,
    message_index: usize,
) -> PyResult<PythonLogOptions<'py>> {
    if args.len() != expected_positional_count {
        return Err(PyTypeError::new_err(format!(
            "expected {expected_positional_count} positional arguments, received {}",
            args.len()
        )));
    }

    let message = args.get_item(message_index)?.extract()?;
    let mut exc_info = None;
    let mut stack_info = false;

    if let Some(keyword_args) = kwargs {
        for (key, value) in keyword_args {
            let keyword = key.extract::<PyBackedStr>()?;
            match keyword.as_str() {
                "exc_info" => {
                    exc_info = (!value.is_none()).then_some(value);
                }
                "stack_info" => {
                    stack_info = value.extract::<Option<bool>>()?.unwrap_or(false);
                }
                _ => {
                    return Err(PyTypeError::new_err(format!(
                        "got an unexpected keyword argument '{keyword}'"
                    )));
                }
            }
        }
    }

    Ok(PythonLogOptions {
        message,
        exc_info,
        stack_info,
    })
}

/// Determine whether `exc_info` should trigger exception capture.
///
/// Returns `true` for any non-False, non-None value (including exception
/// instances and 3-tuples). The actual type validation happens in
/// [`capture_exception`], which will raise `TypeError` for invalid types.
///
/// Returns `false` for `False` or `None` values—these explicitly disable
/// exception capture.
#[cfg(feature = "python")]
pub fn should_capture_exc_info(exc_info: &Bound<'_, PyAny>) -> bool {
    // Handle boolean False explicitly
    if let Ok(b) = exc_info.cast::<PyBool>() {
        return b.is_true();
    }
    // None means no capture
    if exc_info.is_none() {
        return false;
    }
    // Any other value (exception instance, tuple, or invalid type) triggers
    // capture attempt. Invalid types will fail in capture_exception.
    true
}

/// Capture an exception payload when `exc_info` is provided and truthy.
///
/// Returns `Ok(None)` when `exc_info` is absent, falsy, or yields no
/// captured exception.
#[cfg(feature = "python")]
pub(super) fn capture_exception_payload(
    py: Python<'_>,
    exc_info: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<crate::ExceptionPayload>> {
    let Some(exc) = exc_info else {
        return Ok(None);
    };
    if !should_capture_exc_info(exc) {
        return Ok(None);
    }
    crate::traceback_capture::capture_exception(py, exc)
}
