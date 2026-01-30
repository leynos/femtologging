#![cfg(feature = "python")]
//! Integration test for name-based filter application via ConfigBuilder.

use _femtologging_rs::{
    ConfigBuilder, FemtoLevel, LoggerConfigBuilder, NameFilterBuilder, manager,
};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;

/// A name filter should only allow records whose logger name matches the prefix.
#[rstest]
#[serial(manager)]
fn name_filter_blocks_non_matching_records() {
    Python::with_gil(|py| {
        manager::reset_manager();
        let filter = NameFilterBuilder::new().with_prefix("allowed");
        let builder = ConfigBuilder::new()
            .with_filter("n", filter.into())
            .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Debug))
            .with_logger("allowed", LoggerConfigBuilder::new().with_filters(["n"]))
            .with_logger("blocked", LoggerConfigBuilder::new().with_filters(["n"]));
        builder.build_and_init().expect("build should succeed");
        let ok = manager::get_logger(py, "allowed").expect("get_logger('allowed') should succeed");
        let bad = manager::get_logger(py, "blocked").expect("get_logger('blocked') should succeed");
        assert!(ok.borrow(py).log(FemtoLevel::Info, "ok").is_some());
        assert!(bad.borrow(py).log(FemtoLevel::Info, "blocked").is_none());
    });
}
