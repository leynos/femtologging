//! Handler builders and associated traits.
//!
//! Provides a minimal builder API for constructing handlers in a
//! type‑safe manner. Each builder implements [`HandlerBuilderTrait`]
//! which returns a boxed [`FemtoHandlerTrait`] ready for registration
//! with a logger.

use std::io;

use pyo3::create_exception;
use thiserror::Error;

use crate::handler::FemtoHandlerTrait;

mod common;
pub mod file;
pub mod file_builder;
mod formatter_id;
pub mod rotating;
pub mod rotating_builder;
pub mod stream_builder;
#[cfg(test)]
pub mod test_helpers;

pub use file_builder::FileHandlerBuilder;
pub use formatter_id::FormatterId;
pub use rotating::FemtoRotatingFileHandler;
pub use rotating_builder::RotatingFileHandlerBuilder;
pub use stream_builder::StreamHandlerBuilder;

// Define module-level Python exceptions for explicit handling on the Python side.
create_exception!(
    _femtologging_rs,
    HandlerConfigError,
    pyo3::exceptions::PyException
);
create_exception!(
    _femtologging_rs,
    HandlerIOError,
    pyo3::exceptions::PyException
);

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
        match err {
            HandlerBuildError::InvalidConfig(msg) => HandlerConfigError::new_err(msg),
            HandlerBuildError::Io(ioe) => HandlerIOError::new_err(ioe.to_string()),
        }
    }
}

/// Trait implemented by all handler builders.
///
/// Builders return boxed [`FemtoHandlerTrait`] objects so the caller can
/// register them without knowing the concrete handler type.
pub trait HandlerBuilderTrait: Send + Sync {
    type Handler: FemtoHandlerTrait;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError>;

    /// Build the handler instance, returning a boxed trait object.
    fn build(&self) -> Result<Box<dyn FemtoHandlerTrait>, HandlerBuildError> {
        let handler = self.build_inner()?;
        Ok(Box::new(handler))
    }
}
