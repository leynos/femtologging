//! Configuration builders for femtologging.

#[cfg(feature = "python")]
mod build;
#[cfg(feature = "python")]
mod py;
mod types;

#[cfg_attr(
    not(feature = "python"),
    expect(unused_imports, reason = "public re-exports for external consumers")
)]
// Re-export for external consumers
pub use types::{ConfigBuilder, ConfigError, FormatterBuilder, LoggerConfigBuilder};

#[cfg(all(test, feature = "python"))]
mod config_tests;
#[cfg(all(test, feature = "python"))]
mod propagate_tests;
#[cfg(all(test, feature = "python"))]
mod test_utils;
