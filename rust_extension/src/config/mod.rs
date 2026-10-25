//! Configuration builders for femtologging.

#[cfg(feature = "python")]
use once_cell::sync::Lazy;
#[cfg(feature = "python")]
use parking_lot::{Mutex, MutexGuard};

#[cfg(feature = "python")]
mod build;
mod formatter_builder;
mod handler_builder;
#[cfg(feature = "python")]
mod py;
#[cfg(feature = "python")]
mod reachability;
#[cfg(feature = "python")]
mod runtime_mutation;
mod types;

/// Serializes runtime planning and publication across configuration paths.
#[cfg(feature = "python")]
static RUNTIME_TRANSACTION: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Serialize runtime-state planning and publication across configuration paths.
#[cfg(feature = "python")]
pub(crate) fn lock_runtime_transaction() -> MutexGuard<'static, ()> {
    RUNTIME_TRANSACTION.lock()
}
// Re-export for external consumers.
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
pub use types::{ConfigBuilder, ConfigError, HandlerBuilder, LoggerConfigBuilder};

#[cfg(all(test, feature = "python"))]
mod build_tests;
#[cfg(all(test, feature = "python"))]
mod config_tests;
#[cfg(all(test, feature = "python"))]
mod propagate_tests;
#[cfg(all(test, feature = "python"))]
mod reachability_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_lifecycle_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_mutation_tests;
#[cfg(all(test, feature = "python"))]
mod runtime_pruning_tests;
#[cfg(all(test, feature = "python"))]
mod test_utils;
