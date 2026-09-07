//! Unit tests for logger propagation behaviour.
#![cfg(all(test, feature = "python"))]

use super::test_utils::gil_and_clean_manager;
use super::*;
use crate::manager;
use crate::{FemtoLevel, FemtoLogger, FileHandlerBuilder};
use pyo3::{Py, Python};
use rstest::{fixture, rstest};
use serial_test::serial;
use std::fs;
use tempfile::NamedTempFile;

#[fixture]
fn new_root_file_handler() -> std::io::Result<(FileHandlerBuilder, NamedTempFile)> {
    let file = NamedTempFile::new()?;
    let builder = FileHandlerBuilder::new(file.path());
    Ok((builder, file))
}

fn read_log_file(file: &NamedTempFile) -> std::io::Result<String> {
    fs::read_to_string(file.path())
}

/// Assert that flushing `$logger` (bound to `$py`) succeeds, panicking with
/// a message naming `$name` from the caller's line on failure.
macro_rules! assert_flush {
    ($py:expr, $logger:expr, $name:expr) => {{
        let flushed = $logger.borrow($py).flush_handlers();
        assert!(flushed, "{} flush should succeed", $name);
    }};
}

/// Build a root logger with `$root_handler` and a "child" logger under it,
/// then run `build_and_init` and fetch both loggers by name.
///
/// Fallible so the calling test can `.expect()` at the panic boundary.
fn build_root_and_child(
    py: Python<'_>,
    root_handler: FileHandlerBuilder,
    child_cfg: LoggerConfigBuilder,
) -> pyo3::PyResult<(Py<FemtoLogger>, Py<FemtoLogger>)> {
    let root_config = LoggerConfigBuilder::new()
        .with_level(FemtoLevel::Info)
        .with_handlers(["h"]);
    let builder = ConfigBuilder::new()
        .with_handler("h", root_handler)
        .with_root_logger(root_config)
        .with_logger("child", child_cfg);
    builder
        .build_and_init()
        .map_err(|err| pyo3::exceptions::PyRuntimeError::new_err(err.to_string()))?;
    let child = manager::get_logger(py, "child")?;
    let root_logger = manager::get_logger(py, "root")?;
    Ok((child, root_logger))
}

#[rstest]
#[serial]
fn propagate_flag_applied(
    gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
    let () = gil_and_clean_manager;
    Python::attach(|py| {
        let (root_handler, file) = new_root_file_handler.expect("create temp log file");
        let child_cfg = LoggerConfigBuilder::new()
            .with_level(FemtoLevel::Info)
            .with_propagate(false);
        let (child, root) = build_root_and_child(py, root_handler, child_cfg)
            .expect("build and lookup should succeed");
        assert!(child.borrow(py).handlers_for_test().is_empty());
        child.borrow(py).log(FemtoLevel::Info, "msg");
        assert_flush!(py, child, "child");
        assert_flush!(py, root, "root");
        assert!(
            read_log_file(&file)
                .expect("test log file must be readable")
                .is_empty(),
            "root handler should receive no records"
        );
    });
}

#[rstest]
#[serial]
fn record_propagates_to_root(
    gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
    let () = gil_and_clean_manager;
    Python::attach(|py| {
        let (root_handler, file) = new_root_file_handler.expect("create temp log file");
        let child_cfg = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let (child, root) = build_root_and_child(py, root_handler, child_cfg)
            .expect("build and lookup should succeed");
        child.borrow(py).log(FemtoLevel::Info, "msg");
        assert_flush!(py, child, "child");
        assert_flush!(py, root, "root");
        let contents = read_log_file(&file).expect("test log file must be readable");
        assert!(
            contents.contains("msg"),
            "root handler should receive one record"
        );
    });
}

#[rstest]
#[serial]
fn propagate_toggle_runtime(
    gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
    let () = gil_and_clean_manager;
    Python::attach(|py| {
        let (root_handler, file) = new_root_file_handler.expect("create temp log file");
        let child_cfg = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let (child, root) = build_root_and_child(py, root_handler, child_cfg)
            .expect("build and lookup should succeed");
        child.borrow(py).set_propagate(false);
        child.borrow(py).log(FemtoLevel::Info, "one");
        assert_flush!(py, child, "child");
        assert_flush!(py, root, "root");
        assert!(
            !read_log_file(&file)
                .expect("test log file must be readable")
                .contains("one"),
            "records should not propagate when disabled"
        );
        child.borrow(py).set_propagate(true);
        child.borrow(py).log(FemtoLevel::Info, "two");
        assert_flush!(py, child, "child");
        assert_flush!(py, root, "root");
        assert!(
            read_log_file(&file)
                .expect("test log file must be readable")
                .contains("two"),
            "record should propagate after enabling"
        );
    });
}
