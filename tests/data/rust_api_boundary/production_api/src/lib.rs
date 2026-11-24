//! Control probe: a default-feature consumer can name the production API.
//!
//! This crate exists only to compile. It proves that the negative probe fails
//! because the test-only seam is absent, not because the production surface
//! the probes share has broken.

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FileHandlerBuilder, HandlerConfig, OverflowPolicy,
};

/// Return the documented production configuration for a file handler.
pub fn handler_config() -> HandlerConfig {
    HandlerConfig {
        capacity: 8,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Drop,
    }
}

/// Build a production file-handler builder over `path`.
pub fn handler_builder(path: &str) -> FileHandlerBuilder {
    FileHandlerBuilder::new(path).with_formatter(DefaultFormatter)
}

/// Name the handler type that the production builder produces.
pub fn handler_type_name() -> &'static str {
    std::any::type_name::<FemtoFileHandler>()
}