//! Enablement-decision tests for the Rust `log` bridge.
//!
//! [`FemtoLogAdapter::enabled`] is the facade's half of the threshold agreement
//! this issue is about: it must accept exactly the records the target logger
//! would accept after dispatch, without creating a logger or touching Python.

use log::{Log, Metadata};
use pyo3::prelude::*;
use rstest::rstest;
use serial_test::serial;

use super::test_support::{log_max_level, unique_logger_name};
use super::{FemtoLevel, FemtoLogAdapter};
use crate::manager;

/// Enablement follows the live level of a registered target logger.
#[rstest]
#[serial]
fn enabled_uses_the_registered_target_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    for (level, expected) in [
        (log::Level::Trace, false),
        (log::Level::Debug, false),
        (log::Level::Info, false),
        (log::Level::Warn, true),
        (log::Level::Error, true),
    ] {
        let metadata = Metadata::builder()
            .level(level)
            .target(&unique_logger_name)
            .build();
        assert_eq!(adapter.enabled(&metadata), expected, "level {level}");
    }
}

/// Enablement normalizes module targets without creating unknown loggers.
#[rstest]
#[serial]
fn enabled_normalizes_module_targets_and_leaves_unknown_targets_unregistered(
    _log_max_level: (),
    unique_logger_name: String,
) {
    let adapter = FemtoLogAdapter;
    let rust_target = unique_logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    let normalized_metadata = Metadata::builder()
        .level(log::Level::Info)
        .target(&rust_target)
        .build();
    assert!(!adapter.enabled(&normalized_metadata));

    let unknown_target = format!("{unique_logger_name}.missing");
    assert_eq!(manager::registered_logger_level(&unknown_target), None);
    let unknown_metadata = Metadata::builder()
        .level(log::Level::Info)
        .target(&unknown_target)
        .build();
    assert!(adapter.enabled(&unknown_metadata));
    assert_eq!(manager::registered_logger_level(&unknown_target), None);
}

/// Invalid targets use the root threshold, or the default when root is absent.
#[rstest]
#[serial]
fn enabled_uses_root_for_invalid_targets_and_default_when_root_is_absent(_log_max_level: ()) {
    let adapter = FemtoLogAdapter;
    manager::reset_manager();

    let invalid_info = Metadata::builder()
        .level(log::Level::Info)
        .target("invalid..target")
        .build();
    assert!(adapter.enabled(&invalid_info));
    assert_eq!(manager::registered_logger_level("root"), None);

    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root logger created");
        root.borrow(py).set_level(FemtoLevel::Warn);
    });

    let invalid_debug = Metadata::builder()
        .level(log::Level::Debug)
        .target("invalid..target")
        .build();
    let invalid_warn = Metadata::builder()
        .level(log::Level::Warn)
        .target("invalid..target")
        .build();
    assert!(!adapter.enabled(&invalid_debug));
    assert!(!adapter.enabled(&invalid_info));
    assert!(adapter.enabled(&invalid_warn));
    assert_eq!(
        manager::registered_logger_level("root"),
        Some(FemtoLevel::Warn)
    );
}
