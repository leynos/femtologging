//! Timed rotating file handler module wiring.
//!
//! Core timed rotation logic stays in Rust-only modules while Python bindings
//! remain feature-gated.

mod clock;
mod core;
mod schedule;

pub use core::FemtoTimedRotatingFileHandler;
pub use schedule::{TimedRotationSchedule, TimedRotationWhen};

#[cfg(feature = "python")]
pub(crate) mod python;
#[cfg(feature = "python")]
pub(crate) use python::PyTimedRotatingFileHandler;
#[cfg(feature = "python")]
pub use python::{
    TIMED_ROTATION_VALIDATION_MSG, TimedHandlerOptions, clear_timed_rotation_test_times_for_test,
    set_timed_rotation_test_times_for_test,
};
