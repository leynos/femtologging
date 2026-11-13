//! Flush-failure reporting must never repeat handler exception text.
//!
//! A Python handler's exception message is user data and may quote a record,
//! so the flush path reports only a bounded category. These tests drive the
//! real path with an exception carrying a marker and assert the marker reaches
//! neither the captured warning nor stderr.

use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::logger::PyHandler;

use super::FlushFailure;

/// Marker that only the raising handler's exception carries.
const SENSITIVE_MARKER: &str = "SENSITIVE-flush-payload-7d1c";

/// Run `body` with Python's `sys.stderr` replaced by a buffer.
fn with_captured_stderr<T>(py: Python<'_>, body: impl FnOnce() -> T) -> (T, String) {
    let contextlib = py.import("contextlib").expect("contextlib is importable");
    let io = py.import("io").expect("io is importable");
    let buffer = io
        .getattr("StringIO")
        .expect("io.StringIO exists")
        .call0()
        .expect("StringIO constructs");
    let redirect = contextlib
        .getattr("redirect_stderr")
        .expect("contextlib.redirect_stderr exists")
        .call1((&buffer,))
        .expect("the redirect context constructs");
    redirect
        .call_method0("__enter__")
        .expect("the redirect context enters");

    let result = body();

    // `__exit__` always receives the exception triple; `None` reports that the
    // body raised nothing, which is the only case this helper supports.
    redirect
        .call_method1("__exit__", (py.None(), py.None(), py.None()))
        .expect("the redirect context exits");
    let captured = buffer
        .call_method0("getvalue")
        .expect("the buffer is readable")
        .extract::<String>()
        .expect("the buffer holds text");
    (result, captured)
}

/// Build a handler whose `flush` raises an exception carrying the marker.
fn raising_handler(py: Python<'_>) -> PyHandler {
    let module = PyModule::from_code(
        py,
        c"class Raising:\n    def flush(self):\n        raise RuntimeError('SENSITIVE-flush-payload-7d1c')\n",
        c"raising_flush_handler.py",
        c"raising_flush_handler",
    )
    .expect("the raising handler module compiles");
    let object = module
        .getattr("Raising")
        .expect("the class exists")
        .call0()
        .expect("the handler constructs");
    PyHandler::new(py, object.unbind())
}

#[test]
fn a_raising_flush_reports_its_category_without_the_exception_text() {
    install_test_logger();

    Python::attach(|py| {
        let handler = raising_handler(py);
        let (flushed, stderr) = with_captured_stderr(py, || handler.call_python_flush(py));

        assert!(!flushed, "a raising flush must report failure");
        assert!(
            !stderr.contains(SENSITIVE_MARKER),
            "the flush path must not print the exception: {stderr}",
        );
    });

    let warnings: Vec<String> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.level == log::Level::Warn)
        .map(|entry| entry.message)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|message| message == FlushFailure::CallFailed.message()),
        "the flush must report its bounded category: {warnings:?}",
    );
    assert!(
        warnings
            .iter()
            .all(|message| !message.contains(SENSITIVE_MARKER)),
        "no warning may repeat the handler's exception text: {warnings:?}",
    );
}
