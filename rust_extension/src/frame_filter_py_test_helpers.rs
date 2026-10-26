//! Shared configuration and binding helpers for frame-filter tests.

use super::*;

/// Frame-filter options used by Rust binding tests.
#[derive(Default)]
pub(super) struct TestFilterOptions {
    /// Filename fragments whose matching frames should be removed.
    pub(super) exclude_filenames: Option<Vec<String>>,
    /// Function-name fragments whose matching frames should be removed.
    pub(super) exclude_functions: Option<Vec<String>>,
    /// Maximum number of most-recent frames to retain per payload.
    pub(super) max_depth: Option<usize>,
    /// Whether logging-infrastructure frames should be removed.
    pub(super) exclude_logging: bool,
}

/// Call the binding with the default exception-graph budgets for a test.
pub(super) fn filter_frames_with_defaults<'py>(
    py: Python<'py>,
    payload: &Bound<'py, PyDict>,
    options: TestFilterOptions,
) -> PyResult<Py<PyAny>> {
    filter_frames(
        py,
        payload,
        options.exclude_filenames,
        options.exclude_functions,
        options.max_depth,
        options.exclude_logging,
        DEFAULT_MAX_EXCEPTION_DEPTH,
        DEFAULT_MAX_EXCEPTION_NODES,
    )
}
