//! Timed rotating file handler module wiring.
//!
//! Core timed rotation logic stays in Rust-only modules while Python bindings
//! remain feature-gated.
//!
//! The pieces are split by reason to change rather than by layer:
//!
//! - [`handler`] is the public wrapper that attaches a rotation strategy to a
//!   file handler, and [`core`] holds the constructor bundle it is built from.
//! - [`rotation_strategy`] orchestrates one rotation and decides when the
//!   cadence says one is due.
//! - [`rollover_schedule`] owns the clock and the single rollover deadline,
//!   while [`schedule`] holds the cadence rules themselves.
//! - [`rotation_steps`] performs the rename and writer swap and unwinds them on
//!   failure; [`rotation_store`] is the filesystem seam those steps go through.
//! - [`backup_retention`] prunes timestamped siblings after a completed
//!   rotation.
//! - [`clock`] supplies the rollover clock type.
//!
//! The Python-facing modules (`python`, `python_time`) are declared only under
//! the `python` feature, so the rotation core builds and tests without an
//! interpreter.

mod backup_retention;
mod clock;
mod core;
#[cfg(test)]
mod core_tests;
mod handler;
mod rollover_schedule;
mod rotation_steps;
mod rotation_store;
mod rotation_strategy;
#[cfg(test)]
mod rotation_strategy_retention_tests;
#[cfg(test)]
mod rotation_strategy_store_tests;
mod schedule;
#[cfg(test)]
mod schedule_tests;
#[cfg(test)]
mod test_helpers;

pub(crate) use core::TimedRotationConfig;
pub use handler::FemtoTimedRotatingFileHandler;
pub use schedule::{TimedRotationSchedule, TimedRotationWhen};

#[cfg(feature = "python")]
pub(crate) mod python;
#[cfg(feature = "python")]
mod python_time;
#[cfg(feature = "python")]
pub(crate) use python::PyTimedRotatingFileHandler;
#[cfg(feature = "python")]
pub use python::{TIMED_ROTATION_VALIDATION_MSG, TimedHandlerOptions};
#[cfg(all(feature = "python", feature = "test-util"))]
pub use python::{
    clear_timed_rotation_test_times_for_test, set_timed_rotation_test_times_for_test,
};