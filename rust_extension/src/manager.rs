//! Global registry mapping logger names to instances.
//!
//! Access is guarded by a `parking_lot::RwLock` and must only occur while the
//! Python GIL is held. This ensures `Py<FemtoLogger>` objects remain valid.
//!
//! The registry is bounded on two axes. A name is validated against
//! [`MAX_LOGGER_NAME_BYTES`], [`MAX_LOGGER_NAME_SEGMENTS`] and a ban on
//! line-breaking characters before it can become a key, and the registry
//! retains at most [`MAX_LOGGERS`] distinct loggers. Both bounds exist because
//! a logger name is often derived from caller-supplied data; without them a
//! request identifier becomes an unbounded registry key.
//!
//! `"root"` is exempt from the cap: it is the fallback destination the Rust
//! bridges resolve to, so the registry must always be able to hold it.
//!
//! The two concerns live in submodules — [`validation`] owns the name rules
//! and [`registry`] owns the map and the cap — while this module re-exports
//! them as the one path callers already use.

mod registry;
mod validation;

pub(super) use registry::MANAGER;

pub use registry::{MAX_LOGGERS, get_logger, reset_manager};
pub use validation::{MAX_LOGGER_NAME_BYTES, MAX_LOGGER_NAME_SEGMENTS, validate_logger_name};

#[cfg(feature = "log-compat")]
pub(crate) use registry::REGISTRY_FULL_MARKER;

#[cfg(feature = "python")]
pub(crate) use registry::{
    LoggerAttachmentState, RuntimeStateSnapshot, commit_staged_loggers, disable_existing_loggers,
    ensure_registry_capacity, replace_runtime_state, snapshot_runtime_state, stage_loggers,
};

#[cfg(all(test, feature = "python"))]
pub(crate) use registry::lookup_existing_logger;
// Only the tracing bridge's tests reach the counter through this path; without
// `tracing-compat` the re-export would be an unused item in a `-D warnings` lane.
#[cfg(all(test, feature = "tracing-compat"))]
pub(crate) use registry::named_logger_count_for_test;

#[cfg(feature = "log-compat")]
pub(crate) use registry::flush_all_handlers;

#[cfg(test)]
mod manager_tests;
#[cfg(all(test, feature = "python"))]
#[path = "manager/manager_tests_capacity.rs"]
mod manager_tests_capacity;
