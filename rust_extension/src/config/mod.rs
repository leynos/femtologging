//! Configuration builders for femtologging.

#[cfg(feature = "python")]
mod build;
#[cfg(feature = "python")]
mod py;
#[cfg(feature = "python")]
mod runtime_mutation;
mod types;

#[cfg(feature = "python")]
pub use runtime_mutation::{LoggerMutationBuilder, RuntimeConfigBuilder};
// Re-export for external consumers
#[cfg(feature = "python")]
pub use types::ConfigError;
pub use types::{ConfigBuilder, FormatterBuilder, LoggerConfigBuilder};

#[cfg(all(test, feature = "python"))]
mod config_tests;
#[cfg(all(test, feature = "python"))]
mod propagate_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_mutation_tests;
#[cfg(all(test, feature = "python"))]
mod test_utils;
