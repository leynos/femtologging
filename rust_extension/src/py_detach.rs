//! Detaching the interpreter for a Python-facing wait.
//!
//! The native-handler flush and the socket and HTTP `flush()` and `close()`
//! waits go through here, so the test-only in-flight record has one entry
//! point and a new wait of that kind cannot silently omit it. The file,
//! stream and rotating families bracket a direct `Python::detach` with
//! `WaitScope` instead. See [`crate::gil_release_test`] for what the tests do
//! with that record.

use pyo3::marker::Ungil;
use pyo3::prelude::*;

/// Run `wait` with the interpreter detached, recording the span in tests.
///
/// The in-flight guard is entered while the GIL is still held and dropped
/// after `Python::detach` returns, so a test thread's observation can only be
/// counted between the two. That is what lets a test tell the native wait
/// apart from a detached section in front of it. Production builds compile the
/// guard out, so this is `Python::detach` and nothing else.
///
/// # Examples
///
/// A binding waits for its worker without holding the interpreter:
///
/// ```ignore
/// fn py_flush(&self, py: Python<'_>) -> bool {
///     detach_for_wait(py, || self.flush())
/// }
/// ```
pub(crate) fn detach_for_wait<T, F>(py: Python<'_>, wait: F) -> T
where
    F: Ungil + FnOnce() -> T,
    T: Ungil,
{
    #[cfg(all(feature = "python", feature = "test-util"))]
    let _in_flight = crate::gil_release_test::WaitInFlight::enter();
    py.detach(wait)
}
