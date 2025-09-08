//! Configuration builders for femtologging.

mod build;
mod py;
mod types;

#[allow(unused_imports)] // re-export for external consumers
pub use types::{ConfigBuilder, ConfigError, FormatterBuilder, LoggerConfigBuilder};

#[cfg(test)]
mod config_tests;
