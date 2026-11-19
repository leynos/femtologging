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

use pyo3::prelude::*;

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
    // Each failure names the step and the Python error, so a broken redirect
    // is diagnosed from the message alone. `unwrap_or_else(|err| panic!(..))`
    // would read as a panicking closure to Whitaker's `no_unwrap_or_else_panic`,
    // which is why the formatted detail lives in a `match` arm instead.
    let contextlib = match py.import("contextlib") {
        Ok(module) => module,
        Err(err) => panic!("contextlib is importable: {err}"),
    };
    let io = match py.import("io") {
        Ok(module) => module,
        Err(err) => panic!("io is importable: {err}"),
    };
    let buffer = match io.getattr("StringIO").and_then(|factory| factory.call0()) {
        Ok(buffer) => buffer,
        Err(err) => panic!("io.StringIO constructs: {err}"),
    };
    let redirect = match contextlib
        .getattr("redirect_stderr")
        .and_then(|factory| factory.call1((&buffer,)))
    {
        Ok(redirect) => redirect,
        Err(err) => panic!("contextlib.redirect_stderr constructs: {err}"),
    };
    if let Err(err) = redirect.call_method0("__enter__") {
        panic!("the redirect context enters: {err}");
    }

    let result = body();

    // `__exit__` always receives the exception triple; `None` reports that the
    // body raised nothing, which is the only case this helper supports.
    if let Err(err) = redirect.call_method1("__exit__", (py.None(), py.None(), py.None())) {
        panic!("the redirect context exits: {err}");
    }
    let captured = match buffer.call_method0("getvalue") {
        Ok(value) => match value.extract::<String>() {
            Ok(text) => text,
            Err(err) => panic!("the buffer holds text: {err}"),
        },
        Err(err) => panic!("the buffer is readable: {err}"),
    };
    (result, captured)
}
