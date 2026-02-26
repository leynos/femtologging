//! Rotating file handler module wiring.
//!
//! Core rotation logic lives in [`core`], while Python bindings are isolated in
//! [`python`] behind the `python` feature.

mod core;
mod fresh_failure;
mod strategy;

pub use core::{FemtoRotatingFileHandler, RotationConfig};

#[cfg(test)]
pub(crate) use fresh_failure::force_fresh_failure_once_for_test;

#[cfg(feature = "python")]
pub(crate) mod python;
#[cfg(feature = "python")]
pub use python::{
    HandlerOptions, ROTATION_VALIDATION_MSG, clear_rotating_fresh_failure_for_test,
    force_rotating_fresh_failure_for_test,
};

#[cfg(test)]
mod tests;
