//! Configuration builders for femtologging.

#[cfg(feature = "python")]
mod build;
mod formatter_builder;
#[cfg(feature = "python")]
mod py;
#[cfg(feature = "python")]
mod runtime_mutation;
mod types;

// Re-export for external consumers
pub use formatter_builder::FormatterBuilder;
#[cfg_attr(
    not(feature = "python"),
    expect(unused_imports, reason = "public re-exports for Python-enabled builds")
)]
#[cfg(feature = "python")]
pub use runtime_mutation::{LoggerMutationBuilder, RuntimeConfigBuilder};
pub(crate) use types::normalize_vec;
#[cfg_attr(
    not(feature = "python"),
    expect(unused_imports, reason = "public re-exports for external consumers")
)]
pub use types::{ConfigBuilder, ConfigError, LoggerConfigBuilder};

#[cfg(all(test, feature = "python"))]
mod config_tests;
#[cfg(all(test, feature = "python"))]
mod propagate_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_mutation_tests;
#[cfg(all(test, feature = "python"))]
mod test_utils;
