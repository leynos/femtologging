//! Capture Python's `sys.stderr` while a Rust test body runs.
//!
//! Two suites need the same thing: run some production code that may write
//! through Python's `sys.stderr`, then read the text back without letting it
//! reach the test log. Each kept its own copy of the redirect dance, so the
//! copies could drift apart. One helper keeps them identical.
//!
//! This only sees writes made *through the interpreter*, such as
//! [`PyErr::print`](pyo3::PyErr::print). A `writeln!` to `std::io::stderr()`
//! reaches the process's descriptor 2 directly and is not captured; assert
//! such a message through a formatting helper instead, as the refused-target
//! diagnostics do.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

/// Attach the failed step to a Python error.
///
/// The caller's `?` then keeps one failure path per operation, while the
/// message still names the import or call that went wrong.
fn with_step<T>(step: &str, result: PyResult<T>) -> PyResult<T> {
    result.map_err(|err| PyRuntimeError::new_err(format!("{step}: {err}")))
}

/// Run `body` between entering and leaving `sys.stderr` redirection.
///
/// Kept separate from [`with_captured_stderr`] so the fallible Python calls
/// stay on one `?` path each; the wrapper decides how a failure is reported.
fn capture_body<T>(py: Python<'_>, body: impl FnOnce() -> T) -> PyResult<(T, String)> {
    let contextlib = with_step("contextlib is importable", py.import("contextlib"))?;
    let io = with_step("io is importable", py.import("io"))?;
    let buffer = with_step(
        "io.StringIO constructs",
        io.getattr("StringIO").and_then(|factory| factory.call0()),
    )?;
    let redirect = with_step(
        "contextlib.redirect_stderr constructs",
        contextlib
            .getattr("redirect_stderr")
            .and_then(|factory| factory.call1((&buffer,))),
    )?;
    with_step(
        "the redirect context enters",
        redirect.call_method0("__enter__"),
    )?;

    let result = body();

    // `__exit__` always receives the exception triple; `None` reports that the
    // body raised nothing, which is the only case this helper supports.
    with_step(
        "the redirect context exits",
        redirect.call_method1("__exit__", (py.None(), py.None(), py.None())),
    )?;
    let captured = with_step("the buffer is readable", buffer.call_method0("getvalue"))?;
    let text = with_step("the buffer holds text", captured.extract::<String>())?;
    Ok((result, text))
}

/// Run `body` with Python's `sys.stderr` replaced by a buffer.
///
/// Returns the body's value and everything the body wrote through Python's
/// `sys.stderr` while it ran. Writes to the process's stderr descriptor are
/// outside this helper's reach (see the module documentation). The body must
/// not raise: a raised exception would skip the context manager's `__exit__`
/// and leave `sys.stderr` redirected for later tests, so a panicking body is
/// reported here rather than propagated.
///
/// # Examples
///
/// ```ignore
/// Python::attach(|py| {
///     let (value, stderr) = with_captured_stderr(py, || writeln_diagnostic());
///     assert!(stderr.contains(value));
/// });
/// ```
pub(crate) fn with_captured_stderr<T>(py: Python<'_>, body: impl FnOnce() -> T) -> (T, String) {
    // `unwrap_or_else(|err| panic!(..))` would read as a panicking closure to
    // Whitaker's `no_unwrap_or_else_panic`, so the formatted detail lives in a
    // `match` arm instead.
    match capture_body(py, body) {
        Ok(captured) => captured,
        Err(err) => panic!("stderr capture failed: {err}"),
    }
}
