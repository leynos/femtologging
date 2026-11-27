//! Test module declarations for logger dispatch and producer paths.
//!
//! The `logger` module declares this file under `#[cfg(all(test, not(loom)))]`
//! so its sibling test sources need only one `#[path]` surface. Each entry
//! maps a `logger_tests*`, `producer_*` or `producer_tracing_*` file into
//! the module tree, which keeps `logger/mod.rs` readable while the
//! `#[cfg(not(loom))]` and feature gates apply uniformly. The
//! `logger_tests_flush*` variants stay separate because they time queue
//! barriers, which Loom's channels cannot model.

#[cfg(not(loom))]
use super::*;

// These tests use real threads and `crossbeam_channel`; the heavy lane's models
// cover the Loom-specific synchronization paths.
#[cfg(all(not(loom), feature = "python"))]
#[path = "deferred_capture_py_tests.rs"]
mod deferred_capture_py_tests;
#[cfg(not(loom))]
#[path = "deferred_capture_tests.rs"]
mod deferred_capture_tests;
#[cfg(not(loom))]
#[path = "logger_filter_removal_tests.rs"]
mod logger_filter_removal_tests;
#[cfg(not(loom))]
#[path = "logger_tests.rs"]
mod logger_tests;
#[cfg(not(loom))]
#[path = "logger_tests_helpers.rs"]
mod logger_tests_helpers;
#[cfg(all(not(loom), feature = "python"))]
#[path = "producer_context_tests.rs"]
mod producer_context_tests;
#[cfg(all(not(loom), feature = "python"))]
#[path = "producer_disabled_tests.rs"]
mod producer_disabled_tests;
#[cfg(not(loom))]
#[path = "producer_origin_tests.rs"]
mod producer_origin_tests;
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
#[path = "logger_tests_flush_waits.rs"]
mod logger_tests_flush_waits;
#[cfg(not(loom))]
#[path = "logger_tests_flush_worker.rs"]
mod logger_tests_flush_worker;
