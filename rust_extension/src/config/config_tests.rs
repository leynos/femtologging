//! Unit tests for configuration builders.
#![cfg(all(test, feature = "python"))]

use super::*;
use crate::config::ConfigError;
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::handlers::{HandlerBuildError, HandlerBuilderTrait};
use crate::log_record::FemtoLogRecord;
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
use parking_lot::Mutex;
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::any::Any;
use std::sync::Arc;

#[derive(Clone, Default)]
struct CollectingHandler {
    records: Arc<Mutex<Vec<FemtoLogRecord>>>,
}

impl CollectingHandler {
    fn collected(&self) -> Vec<FemtoLogRecord> {
        self.records.lock().clone()
    }
}

impl FemtoHandlerTrait for CollectingHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.records.lock().push(record);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct CollectingHandlerBuilder {
    handler: CollectingHandler,
}

impl CollectingHandlerBuilder {
    fn new() -> Self {
        Self {
            handler: CollectingHandler::default(),
        }
    }

    fn handle(&self) -> CollectingHandler {
        self.handler.clone()
    }
}

impl HandlerBuilderTrait for CollectingHandlerBuilder {
    type Handler = CollectingHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        Ok(self.handler.clone())
    }
}

#[fixture]
fn gil_and_clean_manager() {
    Python::with_gil(|_| manager::reset_manager());
}

fn builder_with_root(root: LoggerConfigBuilder) -> ConfigBuilder {
    ConfigBuilder::new()
        .with_handler("h", StreamHandlerBuilder::stderr())
        .with_root_logger(root)
}

#[fixture]
fn base_logger_builder() -> (ConfigBuilder, LoggerConfigBuilder) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = builder_with_root(root.clone());
    (builder, root)
}

fn assert_handler_count(py: Python<'_>, name: &str, expected: usize, reason: &str) {
    // Fetch a logger and assert it exposes the expected number of handlers.
    let msg = format!("get_logger('{name}') should succeed");
    let logger = manager::get_logger(py, name).expect(&msg);
    let count = logger.borrow(py).handlers_for_test().len();
    assert_eq!(count, expected, "{}", reason);
}

#[rstest]
fn build_rejects_invalid_version() {
    let builder = ConfigBuilder::new().with_version(2);
    let err = builder
        .build_and_init()
        .expect_err("version 2 should be rejected");
    assert!(matches!(err, ConfigError::UnsupportedVersion(2)));
}

#[rstest]
fn build_rejects_missing_root() {
    let builder = ConfigBuilder::new();
    let err = builder
        .build_and_init()
        .expect_err("root logger is required");
    assert!(matches!(err, ConfigError::MissingRootLogger));
}

fn build_accepts_default_version(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new().with_root_logger(root);
    assert!(builder.build_and_init().is_ok());
}

fn shared_handler_attached_once(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let handler = StreamHandlerBuilder::stderr();
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["h"]);
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", handler)
            .with_root_logger(root)
            .with_logger("first", logger_cfg.clone())
            .with_logger("second", logger_cfg);
        builder.build_and_init().expect("build should succeed");
        let first = manager::get_logger(py, "first").expect("get_logger('first') should succeed");
        let second =
            manager::get_logger(py, "second").expect("get_logger('second') should succeed");
        let h1 = first.borrow(py).handlers_for_test();
        let h2 = second.borrow(py).handlers_for_test();
        assert!(
            Arc::ptr_eq(&h1[0], &h2[0]),
            "handler Arc pointers should be shared"
        );
    });
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
        let child = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
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
        let child = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        child.borrow(py).log(FemtoLevel::Info, "msg");
        let records = collector.collected();
        assert_eq!(records.len(), 1, "root handler should receive one record");
    });
}

#[rstest(kind, add, cfg)]
#[case::handler(
    "handler",
    |b: ConfigBuilder| b.with_handler("exists", StreamHandlerBuilder::stderr()),
    LoggerConfigBuilder::new().with_handlers(["missing"]),
)]
#[case::filter(
    "filter",
    |b: ConfigBuilder| b.with_filter("exists", FilterBuilder::Level(LevelFilterBuilder::new())),
    LoggerConfigBuilder::new().with_filters(["missing"]),
)]
#[serial]
fn unknown_id_rejected(
    _gil_and_clean_manager: (),
    #[case] _kind: &str,
    #[case] add: fn(ConfigBuilder) -> ConfigBuilder,
    #[case] cfg: LoggerConfigBuilder,
) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = add(ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", cfg));
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown id");
    if let ConfigError::UnknownIds(mut ids) = err {
        ids.sort();
        assert_eq!(ids, vec!["missing".to_string()]);
    } else {
        panic!("expected UnknownIds");
    }
}

fn reconfig_with_unknown_filter_preserves_existing_filters(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let builder = ConfigBuilder::new()
            .with_filter("lvl", FilterBuilder::Level(filt))
            .with_root_logger(root.clone())
            .with_logger("core", LoggerConfigBuilder::new().with_filters(["lvl"]));
        builder
            .build_and_init()
            .expect("initial build should succeed");
        let logger = manager::get_logger(py, "core").expect("get_logger('core') should succeed");
        assert!(logger.borrow(py).log(FemtoLevel::Error, "drop").is_none());
        let bad = ConfigBuilder::new()
            .with_root_logger(root)
            .with_logger("core", LoggerConfigBuilder::new().with_filters(["missing"]));
        assert!(bad.build_and_init().is_err());
        assert!(logger
            .borrow(py)
            .log(FemtoLevel::Error, "still drop")
            .is_none());
    });
}

fn unknown_filter_id_rejected(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let logger_cfg = LoggerConfigBuilder::new().with_filters(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown filter id");
    assert!(matches!(err, ConfigError::UnknownIds(ids) if ids == vec!["missing".to_string()]));
}

#[rstest(kind, add, cfg)]
#[case::handler(
    "handler",
    |b: ConfigBuilder| b.with_handler("exists", StreamHandlerBuilder::stderr()),
    LoggerConfigBuilder::new().with_handlers(["missing1","missing2"]),
)]
#[case::filter(
    "filter",
    |b: ConfigBuilder| b.with_filter("exists", FilterBuilder::Level(LevelFilterBuilder::new())),
    LoggerConfigBuilder::new().with_filters(["missing1","missing2"]),
)]
#[serial]
fn multiple_unknown_ids_rejected(
    _gil_and_clean_manager: (),
    #[case] _kind: &str,
    #[case] add: fn(ConfigBuilder) -> ConfigBuilder,
    #[case] cfg: LoggerConfigBuilder,
) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = add(ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", cfg));
    let err = builder.build_and_init().expect_err("should fail");
    if let ConfigError::UnknownIds(mut ids) = err {
        ids.sort();
        assert_eq!(ids, vec!["missing1".to_string(), "missing2".to_string()]);
    } else {
        panic!("expected UnknownIds");
    }
}

#[rstest]
#[serial]
fn duplicate_handler_ids_rejected(_gil_and_clean_manager: ()) {
    let handler = StreamHandlerBuilder::stderr();
    let mut logger_cfg = LoggerConfigBuilder::new();
    logger_cfg.handlers = vec!["h".into(), "h".into()];
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new()
        .with_handler("h", handler)
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for duplicate handler ids");
    assert!(matches!(err, ConfigError::DuplicateHandlerIds(ids) if ids == vec!["h".to_string()]));
}

fn duplicate_filter_ids_rejected(_gil_and_clean_manager: ()) {
    let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Info);
    let mut logger_cfg = LoggerConfigBuilder::new();
    logger_cfg.filters = vec!["f".into(), "f".into()];
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new()
        .with_filter("f", FilterBuilder::Level(filt))
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for duplicate filter ids");
    assert!(matches!(err, ConfigError::DuplicateFilterIds(ids) if ids == vec!["f".to_string()]));
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
            builder = builder.with_logger(name, LoggerConfigBuilder::new().with_handlers(["h"]));
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
        let child = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
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

#[rstest]
#[serial]
fn default_level_configures_root_when_missing_level(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let builder = ConfigBuilder::new()
            .with_handler("stderr", StreamHandlerBuilder::stderr())
            .with_default_level(FemtoLevel::Warn)
            .with_root_logger(LoggerConfigBuilder::new().with_handlers(["stderr"]));
        builder
            .build_and_init()
            .expect("build should apply default root level");

        let root = manager::get_logger(py, "root").expect("root logger should exist");
        assert!(
            root.borrow(py)
                .log(FemtoLevel::Info, "suppressed")
                .is_none(),
            "root should honour the configured default WARN level",
        );
        assert!(
            root.borrow(py).log(FemtoLevel::Error, "emitted").is_some(),
            "records at or above WARN should be emitted",
        );
    });
}

#[rstest]
#[serial]
fn default_level_applies_to_child_loggers(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let child_cfg = LoggerConfigBuilder::new().with_handlers(["console"]);
        let builder = ConfigBuilder::new()
            .with_handler("console", StreamHandlerBuilder::stderr())
            .with_default_level(FemtoLevel::Info)
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_level(FemtoLevel::Warn)
                    .with_handlers(["console"]),
            )
            .with_logger("worker", child_cfg);
        builder
            .build_and_init()
            .expect("build should succeed with default levels");

        let worker = manager::get_logger(py, "worker").expect("worker logger should exist");
        assert!(
            worker
                .borrow(py)
                .log(FemtoLevel::Debug, "suppressed")
                .is_none(),
            "child logger should inherit the default INFO level",
        );
        assert!(
            worker.borrow(py).log(FemtoLevel::Info, "visible").is_some(),
            "records meeting the default level should be emitted",
        );
    });
}
