//! Flush-failure reporting must never repeat handler exception text.
//!
//! A Python handler's exception message is user data and may quote a record,
//! so the flush path reports only a bounded category. These tests drive the
//! real path with an exception carrying a marker and assert the marker reaches
//! neither the reported category nor stderr. They cover both a raising `flush`
//! call and a raising lookup of the `flush` attribute, and keep the
//! `AttributeError` opt-out distinct from a real lookup failure.
//!
//! The categories are read from the flush path's per-thread record rather than
//! the process-wide log buffer: that buffer is shared with every other test in
//! the binary, so draining it here would steal their records and let theirs
//! hide the evidence this test needs.

use std::ffi::CStr;

use pyo3::prelude::*;
use pyo3::types::PyModule;
use rstest::rstest;

use crate::logger::PyHandler;

use super::{FlushFailure, reported};

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

/// Build a handler by instantiating *class_name* from Python *source*.
///
/// The three builders below differ only in the source they compile, so the
/// module loading and construction live here once. Each call compiles its own
/// module, so one shared module name is enough to label a traceback.
fn handler_from_source(py: Python<'_>, source: &CStr, class_name: &str) -> PyHandler {
    let module = PyModule::from_code(py, source, c"flush_handler_test.py", c"flush_handler_test")
        .expect("the test handler module compiles");
    let object = module
        .getattr(class_name)
        .expect("the class exists")
        .call0()
        .expect("the handler constructs");
    PyHandler::new(py, object.unbind())
}

/// Build a handler whose `flush` raises an exception carrying the marker.
fn raising_handler(py: Python<'_>) -> PyHandler {
    handler_from_source(
        py,
        c"class Raising:\n    def flush(self):\n        raise RuntimeError('SENSITIVE-flush-payload-7d1c')\n",
        "Raising",
    )
}

/// Build a handler whose `flush` attribute lookup raises a runtime error.
fn lookup_raising_handler(py: Python<'_>) -> PyHandler {
    handler_from_source(
        py,
        c"class LookupRaising:\n    def __getattr__(self, name):\n        raise RuntimeError('SENSITIVE-flush-payload-7d1c')\n",
        "LookupRaising",
    )
}

/// Build a handler whose missing `flush` raises `AttributeError`, the opt-out.
fn attribute_error_handler(py: Python<'_>) -> PyHandler {
    handler_from_source(
        py,
        c"class LookupOptOut:\n    def __getattr__(self, name):\n        raise AttributeError('SENSITIVE-flush-payload-7d1c')\n",
        "LookupOptOut",
    )
}

/// Drive one flush and report what it did.
///
/// The record is thread-local, so clearing it first only discards categories
/// this thread left behind; a parallel test cannot clear or steal this one's
/// evidence.
fn run_flush(build: impl FnOnce(Python<'_>) -> PyHandler) -> (bool, String, Vec<FlushFailure>) {
    let _ = reported::take();
    Python::attach(|py| {
        let handler = build(py);
        let (flushed, stderr) = with_captured_stderr(py, || handler.call_python_flush(py));
        (flushed, stderr, reported::take())
    })
}

/// Assert the handler's exception text reached neither channel.
fn assert_marker_never_reported(stderr: &str, failures: &[FlushFailure]) {
    assert!(
        !stderr.contains(SENSITIVE_MARKER),
        "the flush path must not print the exception: {stderr}",
    );
    assert!(
        failures
            .iter()
            .all(|failure| !failure.message().contains(SENSITIVE_MARKER)),
        "no reported category may repeat the handler's exception text: {failures:?}",
    );
}

#[rstest]
#[case::raising_flush(raising_handler, FlushFailure::CallFailed)]
#[case::lookup_failure(lookup_raising_handler, FlushFailure::LookupFailed)]
fn a_failed_flush_reports_its_category_without_the_exception_text(
    #[case] build: fn(Python<'_>) -> PyHandler,
    #[case] expected: FlushFailure,
) {
    let (flushed, stderr, failures) = run_flush(build);

    assert!(!flushed, "a failed flush must report failure");
    assert_marker_never_reported(&stderr, &failures);
    assert_eq!(
        failures,
        [expected],
        "the flush must report its own bounded category exactly once",
    );
}

#[test]
fn an_attribute_error_during_lookup_still_opts_out_of_flushing() {
    let (flushed, stderr, failures) = run_flush(attribute_error_handler);

    assert!(
        flushed,
        "an attribute error is how a handler reports it has no flush",
    );
    assert!(
        stderr.is_empty(),
        "an opt-out must not print anything: {stderr}",
    );
    assert!(
        failures.is_empty(),
        "an opt-out is not a failure and must not report a category",
    );
}
