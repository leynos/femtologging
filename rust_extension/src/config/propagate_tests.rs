//! Unit tests for logger propagation behaviour.
#![cfg(all(test, feature = "python"))]

use super::*;
use crate::manager;
use crate::{FemtoLevel, FemtoLogger, FileHandlerBuilder};
use pyo3::{Py, Python};
use rstest::{fixture, rstest};
use serial_test::serial;
use std::fs;
use tempfile::NamedTempFile;

fn new_root_file_handler() -> (FileHandlerBuilder, NamedTempFile) {
    let file = NamedTempFile::new().expect("create temp log file");
    let builder = FileHandlerBuilder::new(file.path());
    (builder, file)
}

fn read_log_file(file: &NamedTempFile) -> String {
    fs::read_to_string(file.path()).expect("test log file must be readable")
}

#[fixture]
fn gil_and_clean_manager() {
    Python::with_gil(|_| manager::reset_manager());
}

fn flush_logger_and_assert(py: Python<'_>, logger: &Py<FemtoLogger>, name: &str) {
    assert!(
        logger.borrow(py).flush_handlers(),
        "{name} flush should succeed"
    );
}

#[rstest]
#[serial]
fn propagate_flag_applied(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let (root_handler, file) = new_root_file_handler();
        let root = LoggerConfigBuilder::new()
            .with_level(FemtoLevel::Info)
            .with_handlers(["h"]);
        let child_cfg = LoggerConfigBuilder::new()
            .with_level(FemtoLevel::Info)
            .with_propagate(false);
        let builder = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(root)
            .with_logger("child", child_cfg);
        builder.build_and_init().expect("build should succeed");
        let child = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        assert!(child.borrow(py).handlers_for_test().is_empty());
        child.borrow(py).log(FemtoLevel::Info, "msg");
        let root = manager::get_logger(py, "root").expect("root logger should exist");
        flush_logger_and_assert(py, &child, "child");
        flush_logger_and_assert(py, &root, "root");
        assert!(
            read_log_file(&file).is_empty(),
            "root handler should receive no records"
        );
    });
}

#[rstest]
#[serial]
fn record_propagates_to_root(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let (root_handler, file) = new_root_file_handler();
        let root = LoggerConfigBuilder::new()
            .with_level(FemtoLevel::Info)
            .with_handlers(["h"]);
        let child_cfg = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(root)
            .with_logger("child", child_cfg);
        builder.build_and_init().expect("build should succeed");
        let child = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        child.borrow(py).log(FemtoLevel::Info, "msg");
        let root = manager::get_logger(py, "root").expect("root logger should exist");
        flush_logger_and_assert(py, &child, "child");
        flush_logger_and_assert(py, &root, "root");
        let contents = read_log_file(&file);
        assert!(
            contents.contains("msg"),
            "root handler should receive one record"
        );
    });
}

#[rstest]
#[serial]
fn propagate_toggle_runtime(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let (root_handler, file) = new_root_file_handler();
        let root = LoggerConfigBuilder::new()
            .with_level(FemtoLevel::Info)
            .with_handlers(["h"]);
        let child_cfg = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(root)
            .with_logger("child", child_cfg);
        builder.build_and_init().expect("build should succeed");
        let child = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        let root = manager::get_logger(py, "root").expect("root logger should exist");
        child.borrow(py).set_propagate(false);
        child.borrow(py).log(FemtoLevel::Info, "one");
        flush_logger_and_assert(py, &child, "child");
        flush_logger_and_assert(py, &root, "root");
        assert!(
            !read_log_file(&file).contains("one"),
            "records should not propagate when disabled"
        );
        child.borrow(py).set_propagate(true);
        child.borrow(py).log(FemtoLevel::Info, "two");
        flush_logger_and_assert(py, &child, "child");
        flush_logger_and_assert(py, &root, "root");
        assert!(
            read_log_file(&file).contains("two"),
            "record should propagate after enabling"
        );
    });
}
