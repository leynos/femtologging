//! Shared test utilities for configuration tests.
#![cfg(all(test, feature = "python"))]

use crate::manager;
use pyo3::Python;
use rstest::fixture;

/// Reset the logger manager before each test to ensure isolation.
#[fixture]
pub fn gil_and_clean_manager() {
    Python::attach(|_| manager::reset_manager());
}

pub mod prune_test_support {
    //! Fixtures and probes shared by the registry-pruning and lifetime suites.
    //!
    //! Both suites start from the same baseline configuration and read the same
    //! published registries back, so the scaffolding lives here rather than being
    //! copied into each. Two copies could drift, which would let a pruning rule
    //! satisfy one suite and silently fail the other.

    use std::sync::Arc;

    use rstest::fixture;
    use tempfile::TempDir;

    use crate::{
        FemtoLevel, StreamHandlerBuilder,
        config::{ConfigBuilder, ConfigError, LoggerConfigBuilder},
        filters::{FilterBuilder, LevelFilterBuilder},
        handler::FemtoHandlerTrait,
        manager,
    };

    use super::gil_and_clean_manager;

    /// Create a temporary directory that is removed when the fixture drops.
    #[fixture]
    pub fn temp_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir must create a temporary directory")
    }

    /// Install a `core` logger whose only handler is the stderr stream handler.
    ///
    /// Every definition declared here is attached to a logger, so the published
    /// registries match the declaration exactly and callers can compare against
    /// a known baseline.
    pub fn install_core_logger() {
        configured_baseline().expect("the default baseline configuration should always build");
    }

    /// Build the stderr-handler baseline, leaving the manager reset first.
    #[fixture]
    pub fn configured_core_logger(_gil_and_clean_manager: ()) -> Result<(), ConfigError> {
        configured_baseline()
    }

    /// Build a root logger plus a `core` logger holding one handler and one filter.
    ///
    /// The filter is a level filter pinned to DEBUG, so the core logger starts
    /// able to emit and a replacement filter can be observed changing behaviour.
    fn configured_baseline() -> Result<(), ConfigError> {
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Debug);
        let filter = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        ConfigBuilder::new()
            .with_handler("stderr", StreamHandlerBuilder::stderr())
            .with_filter("lvl", FilterBuilder::Level(filter))
            .with_root_logger(root)
            .with_logger(
                "core",
                LoggerConfigBuilder::new()
                    .with_handlers(["stderr"])
                    .with_filters(["lvl"]),
            )
            .build_and_init()
    }

    /// Return the number of published handler registry entries.
    pub fn handler_registry_len() -> usize {
        manager::snapshot_runtime_state().handler_registry.len()
    }

    /// Return the published handler registry entry for `id`, if any.
    pub fn published_handler(id: &str) -> Option<Arc<dyn FemtoHandlerTrait>> {
        manager::snapshot_runtime_state()
            .handler_registry
            .get(id)
            .cloned()
    }

    /// Report the published handler registry's size and sorted keys.
    pub fn published_handlers() -> (usize, Vec<String>) {
        let registry = manager::snapshot_runtime_state().handler_registry;
        (registry.len(), registry.keys().cloned().collect())
    }

    /// Report the published filter registry's size and sorted keys.
    pub fn published_filters() -> (usize, Vec<String>) {
        let registry = manager::snapshot_runtime_state().filter_registry;
        (registry.len(), registry.keys().cloned().collect())
    }
}
