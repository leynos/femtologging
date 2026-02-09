//! Payload-building helpers for traceback capture.
//!
//! This submodule isolates TracebackException-to-schema conversion, including
//! chained exceptions and exception-group extraction.

use pyo3::prelude::*;
use pyo3::types::{PyList, PyString, PyTuple};

use crate::exception_schema::{EXCEPTION_SCHEMA_VERSION, ExceptionPayload};
use crate::traceback_frames::{extract_frames_from_tb_exception, get_optional_attr};

/// Build payload from a `TracebackException` object.
///
/// The `exc_value` parameter is the original exception instance. It's optional
/// because for chained exceptions we may not have direct access to the
/// exception instance (only the TracebackException).
pub(super) fn build_payload_from_traceback_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
    exc_value: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<ExceptionPayload>> {
    let (type_name, module) = extract_exception_type_info(tb_exc)?;

    // Extract message
    let message = format_exception_message(tb_exc)?;

    // Extract args_repr from original exception
    let args_repr = if let Some(exc) = exc_value {
        extract_args_repr_from_exc(exc)?
    } else {
        Vec::new()
    };

    // Extract notes from original exception (__notes__ attribute, Python 3.11+)
    let notes = if let Some(exc) = exc_value {
        extract_notes_from_exc(exc)?
    } else {
        Vec::new()
    };

    // Extract stack frames
    let frames = extract_frames_from_tb_exception(tb_exc)?;

    // Handle exception chaining
    let cause = extract_chained_exception(py, tb_exc, "__cause__")?;
    let context = extract_chained_exception(py, tb_exc, "__context__")?;

    // Check suppress_context
    let suppress_context: bool = tb_exc
        .getattr("__suppress_context__")
        .and_then(|v| v.extract())
        .unwrap_or(false);

    // Handle ExceptionGroup (Python 3.11+)
    let exceptions = extract_exception_group(py, tb_exc)?;

    Ok(Some(ExceptionPayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        type_name,
        module,
        message,
        args_repr,
        notes,
        frames,
        cause: cause.map(Box::new),
        context: context.map(Box::new),
        suppress_context,
        exceptions,
    }))
}

/// Normalize the module value for an exception type.
///
/// Built-in exceptions report `"builtins"` as their module, which is not
/// useful for display or grouping. This function returns `None` for that
/// sentinel value and passes all other module strings through unchanged.
fn normalize_module(raw: Option<String>) -> Option<String> {
    raw.filter(|module_name| module_name != "builtins")
}

/// Extract exception type name and module from a `TracebackException`.
///
/// Python 3.13 deprecated the `exc_type` attribute and replaced it with
/// `exc_type_qualname` and `exc_type_module`. This function uses the new
/// attributes when available and falls back to `exc_type` on older versions.
///
/// Returns `(type_name, module)` where `module` is `None` for built-in
/// exceptions.
fn extract_exception_type_info(tb_exc: &Bound<'_, PyAny>) -> PyResult<(String, Option<String>)> {
    // Python 3.13+: use exc_type_qualname / exc_type_module to avoid
    // the DeprecationWarning triggered by accessing exc_type.
    if let Ok(qualname) = tb_exc.getattr("exc_type_qualname") {
        let qualname_str: String = qualname.extract()?;
        // Extract simple name from qualified name (e.g., "Outer.InnerError" → "InnerError")
        let type_name = qualname_str
            .rsplit_once('.')
            .map(|(_, name)| name.to_string())
            .unwrap_or(qualname_str);
        let module = normalize_module(get_optional_attr::<String>(tb_exc, "exc_type_module"));
        return Ok((type_name, module));
    }

    // Python ≤3.12 fallback: read the class object directly.
    let exc_type = tb_exc.getattr("exc_type")?;
    let type_name: String = exc_type.getattr("__name__")?.extract()?;
    let module = normalize_module(
        exc_type
            .getattr("__module__")
            .ok()
            .and_then(|module_name| module_name.extract().ok()),
    );
    Ok((type_name, module))
}

/// Format the exception message from a TracebackException.
fn format_exception_message(tb_exc: &Bound<'_, PyAny>) -> PyResult<String> {
    // _str is the formatted exception message
    let msg = tb_exc.getattr("_str")?;
    if msg.is_none() {
        return Ok(String::new());
    }
    // _str can be a tuple or a string
    if let Ok(tuple) = msg.cast::<PyTuple>() {
        // For exceptions with multiple args, _str is a tuple
        let parts: Vec<String> = tuple
            .iter()
            .filter_map(|item| item.str().ok())
            .filter_map(|string_obj| string_obj.extract().ok())
            .collect();
        return Ok(parts.join(", "));
    }
    msg.str()?.extract()
}

/// Extract args as string representations from the exception instance.
///
/// Per the ADR "partial extraction of collections" rule, individual elements
/// whose `repr()` fails are skipped. Valid representations are preserved.
fn extract_args_repr_from_exc(exc: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let args = match exc.getattr("args") {
        Ok(value) => value,
        Err(_) => return Ok(Vec::new()),
    };
    if args.is_none() {
        return Ok(Vec::new());
    }

    let args_tuple = match args.cast::<PyTuple>() {
        Ok(tuple) => tuple,
        Err(_) => return Ok(Vec::new()),
    };
    let mut result = Vec::with_capacity(args_tuple.len());
    for arg in args_tuple.iter() {
        // Skip elements whose repr() fails per ADR partial extraction rule
        if let Ok(repr_str) = arg.repr().and_then(|repr_obj| repr_obj.extract::<String>()) {
            result.push(repr_str);
        }
    }
    Ok(result)
}

/// Extract exception notes from the exception instance (__notes__).
///
/// Per the ADR "partial extraction of collections" rule, individual elements
/// that are not strings are skipped. Only actual Python `str` objects are
/// included in the result; non-strings are silently ignored.
fn extract_notes_from_exc(exc: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let Some(notes_list): Option<Bound<'_, PyList>> = get_optional_attr(exc, "__notes__") else {
        return Ok(Vec::new());
    };
    let mut result = Vec::with_capacity(notes_list.len());
    for item in notes_list.iter() {
        // Only include actual string objects; skip non-strings per ADR
        if let Ok(string_obj) = item.cast::<PyString>()
            && let Ok(extracted) = string_obj.extract::<String>()
        {
            result.push(extracted);
        }
    }
    Ok(result)
}

/// Extract a chained exception (__cause__ or __context__).
fn extract_chained_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
    attr_name: &str,
) -> PyResult<Option<ExceptionPayload>> {
    let chained = match tb_exc.getattr(attr_name) {
        Ok(chained_exc) if !chained_exc.is_none() => chained_exc,
        _ => return Ok(None),
    };

    // chained is itself a TracebackException
    // We don't have direct access to the chained exception instance here
    build_payload_from_traceback_exception(py, &chained, None)?
        .map(Ok)
        .transpose()
}

/// Extract nested exceptions from an ExceptionGroup.
fn extract_exception_group(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
) -> PyResult<Vec<ExceptionPayload>> {
    // Check if this is an ExceptionGroup by looking for 'exceptions' attribute
    let exceptions_attr = match tb_exc.getattr("exceptions") {
        Ok(value) if !value.is_none() => value,
        _ => return Ok(Vec::new()),
    };

    let exceptions_list = exceptions_attr.cast::<PyList>()?;
    let mut result = Vec::with_capacity(exceptions_list.len());

    for nested_tb_exc in exceptions_list.iter() {
        // We don't have direct access to the nested exception instances here
        if let Some(payload) = build_payload_from_traceback_exception(py, &nested_tb_exc, None)? {
            result.push(payload);
        }
    }

    Ok(result)
}
