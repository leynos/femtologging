//! Python-visible test hooks for rotating handler failure injection.
//!
//! The hooks exist only when the extension is built with both the `python`
//! bindings and the non-default `test-util` feature. Keeping them in their own
//! module leaves the production bindings free of test seams and keeps the
//! `python` binding module under the repository's line limit.

use pyo3::prelude::*;

use super::fresh_failure;

/// Force upcoming file-reopen attempts to fail so retry handling can be tested.
///
/// The supplied reason is attached to each induced failure. When it is `None`,
/// the test helper uses a default reason.
///
/// # Examples
///
/// ```ignore
/// force_rotating_fresh_failure_for_test(2, Some("simulated reopen failure"))?;
/// ```
#[pyfunction]
pub fn force_rotating_fresh_failure_for_test(count: usize, reason: Option<&str>) -> PyResult<()> {
    let reason = reason
        .map(|value| value.to_string())
        .unwrap_or_else(|| "python requested failure".to_string());
    fresh_failure::set_forced_fresh_failure(count, reason);
    Ok(())
}

/// Clear any forced reopen failures installed by a test.
///
/// # Examples
///
/// ```ignore
/// clear_rotating_fresh_failure_for_test();
/// ```
#[pyfunction]
pub fn clear_rotating_fresh_failure_for_test() {
    fresh_failure::clear_forced_fresh_failure();
}
