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
#[cfg(feature = "python")]
mod runtime_state;
mod validation;

pub use registry::{get_logger, reset_manager};
pub use validation::validate_logger_name;

// The bounds are public only when this module is public or rustdoc needs them
// to resolve links. In ordinary builds the manager module is private, so its
// unused public re-exports fail the strict no-default-features library lane.
#[cfg(any(feature = "test-util", doc))]
pub use registry::MAX_LOGGERS;
#[cfg(any(feature = "test-util", doc))]
pub use validation::{MAX_LOGGER_NAME_BYTES, MAX_LOGGER_NAME_SEGMENTS};

// Unit tests use the bounds through the manager path, but keep those imports
// crate-private when the opt-in public test-util API is absent.
#[cfg(all(test, not(feature = "test-util")))]
pub(crate) use registry::MAX_LOGGERS;
#[cfg(all(test, not(feature = "test-util")))]
pub(crate) use validation::MAX_LOGGER_NAME_BYTES;
#[cfg(all(
    test,
    any(feature = "log-compat", feature = "tracing-compat"),
    not(feature = "test-util")
))]
pub(crate) use validation::MAX_LOGGER_NAME_SEGMENTS;

#[cfg(feature = "log-compat")]
pub(crate) use registry::REGISTRY_FULL_MARKER;

// Only the `log` bridge quotes a refused target outside this module; without
// that feature the re-export would be an unused item in a `-D warnings` lane.
#[cfg(feature = "log-compat")]
pub(crate) use validation::describe_name;

#[cfg(feature = "python")]
pub(crate) use registry::{
    commit_staged_loggers, disable_existing_loggers, ensure_registry_capacity,
    replace_runtime_state, snapshot_runtime_state, stage_loggers,
};
#[cfg(feature = "python")]
pub(crate) use runtime_state::{LoggerAttachmentState, RuntimeStateSnapshot};

#[cfg(all(test, feature = "python"))]
pub(crate) use registry::lookup_existing_logger;
// Bridge saturation tests use the named count to assert the cap directly.
#[cfg(all(test, any(feature = "log-compat", feature = "tracing-compat")))]
pub(crate) use registry::named_logger_count_for_test;

#[cfg(feature = "log-compat")]
pub(crate) use registry::flush_all_handlers;

// The `log` bridge reads live thresholds and validates a target without
// taking the manager lock or attaching to Python.
#[cfg(feature = "log-compat")]
pub(crate) use registry::{registered_logger_level, registered_logger_levels};
#[cfg(feature = "log-compat")]
pub(crate) use validation::is_valid_logger_name;

#[cfg(test)]
mod manager_tests;
#[cfg(all(test, feature = "python"))]
#[path = "manager/manager_tests_capacity.rs"]
mod manager_tests_capacity;
