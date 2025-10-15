//! Validation-focused configuration builder tests.
#![cfg(all(test, feature = "python"))]

use super::super::*;
use crate::config::{ConfigBuilder, ConfigError, LoggerConfigBuilder};
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
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
    let builder = add(
        ConfigBuilder::new()
            .with_root_logger(root)
            .with_logger("child", cfg),
    );
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

#[rstest]
#[serial]
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
        let logger =
            manager::get_logger(py, "core").expect("get_logger('core') should succeed");
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

#[rstest]
#[serial]
fn unknown_filter_id_rejected(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let logger_cfg = LoggerConfigBuilder::new().with_filters(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown filter id");
    assert!(matches!(
        err,
        ConfigError::UnknownIds(ids) if ids == vec!["missing".to_string()]
    ));
}

#[rstest(kind, add, cfg)]
#[case::handler(
    "handler",
    |b: ConfigBuilder| b.with_handler("exists", StreamHandlerBuilder::stderr()),
    LoggerConfigBuilder::new().with_handlers(["missing1", "missing2"]),
)]
#[case::filter(
    "filter",
    |b: ConfigBuilder| b.with_filter("exists", FilterBuilder::Level(LevelFilterBuilder::new())),
    LoggerConfigBuilder::new().with_filters(["missing1", "missing2"]),
)]
#[serial]
fn multiple_unknown_ids_rejected(
    _gil_and_clean_manager: (),
    #[case] _kind: &str,
    #[case] add: fn(ConfigBuilder) -> ConfigBuilder,
    #[case] cfg: LoggerConfigBuilder,
) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = add(
        ConfigBuilder::new()
            .with_root_logger(root)
            .with_logger("child", cfg),
    );
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
    assert!(matches!(
        err,
        ConfigError::DuplicateHandlerIds(ids) if ids == vec!["h".to_string()]
    ));
}

#[rstest]
#[serial]
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
    assert!(matches!(
        err,
        ConfigError::DuplicateFilterIds(ids) if ids == vec!["f".to_string()]
    ));
}
