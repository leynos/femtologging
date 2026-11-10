//! Test-only helpers shared across crate unit tests.
//!
//! This module is only compiled for unit tests and provides small utilities
//! used by multiple test modules to keep individual test files focused and
//! below the repository line-length limit.

pub mod collecting_handler;
pub(crate) mod flush_budget;
/// Assertions for extracted traceback frame fields and local variables.
#[cfg(feature = "python")]
pub mod frame_assertion_helpers;
pub mod frame_test_helpers;
pub(crate) mod test_worker;
// Its only callers are the bridge saturation tests, which are excluded under
// `--cfg loom`; keeping the module out of that build avoids a dead-code error
// from the Loom Clippy lane.
#[cfg(all(
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
pub(crate) mod isolated_child;
#[cfg(feature = "python")]
pub mod traceback_test_helpers;
