//! Tests ensuring disable_existing_loggers behaves correctly.
#![cfg(all(test, feature = "python"))]

use super::super::*;
use crate::config::{ConfigBuilder, LoggerConfigBuilder};
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::manager;
use crate::FemtoLevel;
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;

#[fixture]
fn gil_and_clean_manager() {
    super::super::gil_and_clean_manager();
}

#[fixture]
fn base_logger_builder() -> (ConfigBuilder, LoggerConfigBuilder) {
    super::super::base_logger_builder()
}

fn disable_existing_loggers_clears_unmentioned(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::with_gil(|py| {
        let (builder, root) = base_logger_builder;
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let builder = builder
            .with_filter("f", FilterBuilder::Level(filt))
            .with_logger(
                "stale",
                LoggerConfigBuilder::new()
                    .with_handlers(["h"])
                    .with_filters(["f"]),
            );
        builder
            .build_and_init()
            .expect("initial build should succeed");

        assert_handler_count(py, "stale", 1, "stale logger should start active");

        let rebuild = ConfigBuilder::new()
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        assert_handler_count(py, "stale", 0, "stale logger should be disabled");
    });
}

#[rstest(
    ancestor_names,
    case::parent(&["parent"]),
    case::grandparent(&["grandparent", "grandparent.parent"]),
)]
#[serial]
fn disable_existing_loggers_keeps_ancestors(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
    ancestor_names: &[&str],
) {
    Python::with_gil(|py| {
        let (mut builder, root) = base_logger_builder;
        for name in ancestor_names {
            builder =
                builder.with_logger(name, LoggerConfigBuilder::new().with_handlers(["h"]));
        }
        builder
            .build_and_init()
            .expect("initial build should succeed");

        for name in ancestor_names {
            assert_handler_count(py, name, 1, "ancestor logger should start active");
        }

        let child_name = format!(
            "{}.child",
            ancestor_names.last().expect("at least one ancestor")
        );
        let rebuild = builder_with_root(root)
            .with_logger(&child_name, LoggerConfigBuilder::new().with_handlers(["h"]))
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        for name in ancestor_names {
            assert_handler_count(py, name, 1, "ancestor logger should remain active");
        }
        assert_handler_count(py, &child_name, 1, "child logger should retain its handler");

        let logging = py.import("logging").expect("import logging");
        let py_child = logging
            .call_method1("getLogger", (&child_name,))
            .expect("getLogger should succeed");
        let py_handlers = py_child
            .getattr("handlers")
            .expect("child logger should expose handlers");
        assert_eq!(
            py_handlers.len().unwrap(),
            1,
            "child logger should retain its handler",
        );
    });
}
