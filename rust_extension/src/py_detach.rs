//! Detaching the interpreter for a Python-facing wait.
//!
//! Every wait a Python binding performs on a worker goes through here, so the
//! test-only GIL-release signal has exactly one place to be raised from and a
//! new wait cannot silently omit it. See [`crate::gil_release_test`] for what
//! the tests do with that signal.

use pyo3::marker::Ungil;
use pyo3::prelude::*;

/// Run `wait` with the interpreter detached, reporting the release in tests.
///
/// The signal is raised while the GIL is still held, immediately before the
/// detach, which is what lets a test tell a wait apart from a detached section
/// in front of it. Production builds compile the signal out, so this is
/// `Python::detach` and nothing else.
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
    crate::gil_release_test::signal_gil_release();
    py.detach(wait)
}
