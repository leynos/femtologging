//! Tests covering handler propagation behaviour in configuration updates.
#![cfg(all(test, feature = "python"))]

use super::super::*;
use crate::config::{ConfigBuilder, LoggerConfigBuilder};
use crate::manager;
use crate::FemtoLevel;
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;

#[fixture]
fn gil_and_clean_manager() {
    super::super::gil_and_clean_manager();
}

#[rstest]
#[serial]
fn propagate_flag_applied(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let root_handler = CollectingHandlerBuilder::new();
        let collector = root_handler.handle();
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let child_cfg = LoggerConfigBuilder::new()
            .with_level(FemtoLevel::Info)
            .with_propagate(false);
        let builder = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(root)
            .with_logger("child", child_cfg);
        builder.build_and_init().expect("build should succeed");
        let child =
            manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        assert!(child.borrow(py).handlers_for_test().is_empty());
        child.borrow(py).log(FemtoLevel::Info, "msg");
        assert!(
            collector.collected().is_empty(),
            "root handler should receive no records"
        );
    });
}

#[rstest]
#[serial]
fn record_propagates_to_root(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let root_handler = CollectingHandlerBuilder::new();
        let collector = root_handler.handle();
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let child_cfg = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(root)
            .with_logger("child", child_cfg);
        builder.build_and_init().expect("build should succeed");
        let child =
            manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        child.borrow(py).log(FemtoLevel::Info, "msg");
        let records = collector.collected();
        assert_eq!(records.len(), 1, "root handler should receive one record");
    });
}

#[rstest]
#[serial]
fn propagate_toggle_runtime(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let root_handler = CollectingHandlerBuilder::new();
        let collector = root_handler.handle();
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let child_cfg = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(root)
            .with_logger("child", child_cfg);
        builder.build_and_init().expect("build should succeed");
        let child =
            manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        child.borrow(py).set_propagate(false);
        child.borrow(py).log(FemtoLevel::Info, "one");
        assert!(
            collector.collected().is_empty(),
            "records should not propagate when disabled"
        );
        child.borrow(py).set_propagate(true);
        child.borrow(py).log(FemtoLevel::Info, "two");
        let records = collector.collected();
        assert_eq!(records.len(), 1, "record should propagate after enabling");
        assert_eq!(records[0].message, "two");
    });
}
