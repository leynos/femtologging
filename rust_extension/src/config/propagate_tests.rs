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
    let root = LoggerConfigBuilder::new()
        .with_level(FemtoLevel::Info)
        .with_handlers(["h"]);
    let builder = ConfigBuilder::new()
        .with_handler("h", root_handler)
        .with_root_logger(root)
        .with_logger("child", child_cfg);
    builder
        .build_and_init()
        .map_err(|err| pyo3::exceptions::PyRuntimeError::new_err(err.to_string()))?;
    let child = manager::get_logger(py, "child")?;
    let root = manager::get_logger(py, "root")?;
    Ok((child, root))
}

#[rstest]
#[serial]
fn propagate_flag_applied(
    _gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
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
    _gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
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
fn disabled_logger_cannot_reach_root_handler(
    _gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
    Python::attach(|py| {
        let (root_handler, file) = new_root_file_handler.expect("create temp log file");
        // `omitted` is never named by the second configuration, and it keeps
        // the default `propagate = true`. Records must stop at the disabled
        // flag rather than reaching root through propagation.
        let initial = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Info))
            .with_logger(
                "omitted",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Info),
            );
        initial
            .build_and_init()
            .expect("initial build should succeed");

        let omitted = manager::get_logger(py, "omitted").expect("omitted logger exists");
        assert!(
            omitted.borrow(py).propagate(),
            "omitted logger must start with default propagation enabled",
        );

        let rebuild = ConfigBuilder::new()
            .with_handler("h", FileHandlerBuilder::new(file.path()))
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_level(FemtoLevel::Info)
                    .with_handlers(["h"]),
            )
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");
        assert!(
            omitted.borrow(py).is_disabled(),
            "omitted logger should be disabled by the rebuild",
        );

        assert_eq!(
            omitted
                .borrow(py)
                .log(FemtoLevel::Info, "must not reach root"),
            None,
            "a disabled logger must return no formatted message",
        );

        let root = manager::get_logger(py, "root").expect("root logger exists");
        omitted.borrow(py).flush_handlers();
        root.borrow(py).flush_handlers();
        assert!(
            read_log_file(&file)
                .expect("test log file must be readable")
                .is_empty(),
            "an omitted logger must not reach the configured root handler",
        );
    });
}

#[rstest]
#[serial]
fn configured_child_propagates_through_disabled_kept_ancestor(
    _gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
    Python::attach(|py| {
        let (root_handler, file) = new_root_file_handler.expect("create temp log file");
        let initial = ConfigBuilder::new()
            .with_handler("h", root_handler)
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_level(FemtoLevel::Info)
                    .with_handlers(["h"]),
            )
            .with_logger("a", LoggerConfigBuilder::new().with_handlers(["h"]));
        initial
            .build_and_init()
            .expect("initial build should succeed");

        let disable_a = ConfigBuilder::new()
            .with_handler("h", FileHandlerBuilder::new(file.path()))
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_level(FemtoLevel::Info)
                    .with_handlers(["h"]),
            )
            .with_disable_existing_loggers(true);
        disable_a
            .build_and_init()
            .expect("omitting a should disable it");

        let configure_child = ConfigBuilder::new()
            .with_handler("h", FileHandlerBuilder::new(file.path()))
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_level(FemtoLevel::Info)
                    .with_handlers(["h"]),
            )
            .with_logger(
                "a.b",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Info),
            )
            .with_disable_existing_loggers(true);
        configure_child
            .build_and_init()
            .expect("configured descendant should preserve its ancestor");

        let ancestor = manager::get_logger(py, "a").expect("ancestor logger exists");
        let child = manager::get_logger(py, "a.b").expect("child logger exists");
        let root = manager::get_logger(py, "root").expect("root logger exists");
        assert!(
            ancestor.borrow(py).is_disabled(),
            "keeping an ancestor must not re-enable it",
        );
        assert!(
            !child.borrow(py).is_disabled(),
            "the explicitly configured child must be enabled",
        );

        child
            .borrow(py)
            .log(FemtoLevel::Info, "child record reaches root");
        assert_flush!(py, child, "child");
        assert_flush!(py, ancestor, "ancestor");
        assert_flush!(py, root, "root");
        assert!(
            read_log_file(&file)
                .expect("test log file must be readable")
                .contains("child record reaches root"),
            "an enabled child must propagate through a disabled kept ancestor",
        );
    });
}

#[rstest]
#[serial]
fn propagate_toggle_runtime(
    _gil_and_clean_manager: (),
    new_root_file_handler: std::io::Result<(FileHandlerBuilder, NamedTempFile)>,
) {
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
