//! Test-only helpers shared across crate unit tests.
//!
//! This module is only compiled for unit tests and provides small utilities
//! used by multiple test modules to keep individual test files focused and
//! below the repository line-length limit.

/// Arrangements that drive bounded capture against real Python exceptions.
#[cfg(feature = "python")]
pub mod capture_limit_helpers;
pub mod collecting_handler;
pub(crate) mod flush_budget;
pub mod frame_test_helpers;
pub(crate) mod test_worker;
// Every caller is a subprocess-based test that is itself excluded under
// `--cfg loom`; keeping the module out of that build avoids a dead-code error
// from the Loom Clippy lane. The module carries feature-independent helpers
// too, so only the bridge entry point below is feature-gated.
#[cfg(not(loom))]
pub(crate) mod isolated_child;
pub mod json_round_trip;
#[cfg(feature = "python")]
pub mod logging_test_helpers;
// Its only caller is the py-handler call suite, which is excluded under
// `--cfg loom`; keeping the module out of that build avoids the dead-code
// errors the Loom Clippy lane reports for it.
#[cfg(all(feature = "python", not(loom)))]
pub mod stderr_capture;
#[cfg(feature = "python")]
pub mod traceback_test_helpers;
