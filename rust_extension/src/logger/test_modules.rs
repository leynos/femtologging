//! Test module declarations for logger dispatch and producer paths.

use super::*;

// These tests use real threads and `crossbeam_channel`; the heavy lane's models
// cover the Loom-specific synchronization paths.
#[cfg(not(loom))]
#[path = "logger_tests.rs"]
mod logger_tests;
#[cfg(not(loom))]
#[path = "logger_tests_helpers.rs"]
mod logger_tests_helpers;
#[cfg(not(loom))]
#[path = "producer_rejection_tests.rs"]
mod producer_rejection_tests;
#[cfg(not(loom))]
#[path = "producer_tests.rs"]
mod producer_tests;
#[cfg(all(not(loom), feature = "python"))]
#[path = "producer_context_tests.rs"]
mod producer_context_tests;
