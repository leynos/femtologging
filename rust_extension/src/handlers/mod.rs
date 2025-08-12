//! Handler builders and associated traits.
//!
//! Provides a minimal builder API for constructing handlers in a
//! type‑safe manner. Each builder implements [`HandlerBuilderTrait`]
//! which returns a boxed [`FemtoHandlerTrait`] ready for registration
//! with a logger.

use std::io;

use thiserror::Error;

use crate::handler::FemtoHandlerTrait;

mod common;
pub mod file;
pub mod file_builder;
pub mod stream_builder;

pub use file_builder::FileHandlerBuilder;
pub use stream_builder::StreamHandlerBuilder;

/// Errors that may occur while building a handler.
#[derive(Debug, Error)]
pub enum HandlerBuildError {
    /// Invalid user supplied configuration.
    #[error("invalid handler configuration: {0}")]
    InvalidConfig(String),
    /// Underlying I/O error whilst creating the handler.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<HandlerBuildError> for pyo3::PyErr {
    fn from(err: HandlerBuildError) -> Self {
        use pyo3::exceptions::{PyIOError, PyValueError};
        match err {
            HandlerBuildError::InvalidConfig(msg) => PyValueError::new_err(msg),
            HandlerBuildError::Io(ioe) => PyIOError::new_err(ioe.to_string()),
        }
    }
}

/// Trait implemented by all handler builders.
///
/// Builders return boxed [`FemtoHandlerTrait`] objects so the caller can
/// register them without knowing the concrete handler type.
pub trait HandlerBuilderTrait: Send + Sync {
    /// Build the handler instance.
    fn build(&self) -> Result<Box<dyn FemtoHandlerTrait>, HandlerBuildError>;
}
