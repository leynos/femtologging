//! Test module declarations for logger dispatch and producer paths.

#[cfg(not(loom))]
use super::*;

// These tests use real threads and `crossbeam_channel`; the heavy lane's models
// cover the Loom-specific synchronization paths.
#[cfg(not(loom))]
#[path = "logger_tests.rs"]
mod logger_tests;
#[cfg(not(loom))]
#[path = "logger_tests_helpers.rs"]
mod logger_tests_helpers;
#[cfg(all(not(loom), feature = "python"))]
#[path = "producer_context_tests.rs"]
mod producer_context_tests;
#[cfg(not(loom))]
#[path = "producer_rejection_tests.rs"]
mod producer_rejection_tests;
#[cfg(not(loom))]
#[path = "producer_tests.rs"]
mod producer_tests;
#[cfg(all(not(loom), feature = "tracing-compat"))]
#[path = "producer_tracing_tests.rs"]
mod producer_tracing_tests;
// Flush tests time queue barriers; Loom channels intentionally have no clock.
#[cfg(not(loom))]
#[path = "logger_tests_flush.rs"]
mod logger_tests_flush;
#[cfg(not(loom))]
#[path = "logger_tests_flush_deadline.rs"]
mod logger_tests_flush_deadline;
#[cfg(not(loom))]
#[path = "logger_tests_flush_legacy.rs"]
mod logger_tests_flush_legacy;
#[cfg(not(loom))]
#[path = "logger_tests_flush_worker.rs"]
mod logger_tests_flush_worker;
