//! Integration test for logger filter application via ConfigBuilder.

use _femtologging_rs::{
    manager, ConfigBuilder, FemtoLevel, LevelFilterBuilder, LoggerConfigBuilder,
};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;

/// Building a configuration with a level filter should suppress
/// records above the maximum level.
#[rstest]
#[serial]
fn level_filter_blocks_records() {
    Python::with_gil(|py| {
        manager::reset_manager();
        let filter = LevelFilterBuilder::new().with_max_level(FemtoLevel::Info);
        let logger_cfg = LoggerConfigBuilder::new().with_filters(["f"]);
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Debug);
        let builder = ConfigBuilder::new()
            .with_filter("f", filter.into())
            .with_root_logger(root)
            .with_logger("child", logger_cfg);
        builder.build_and_init().expect("build should succeed");
        let logger = manager::get_logger(py, "child").expect("get_logger('child') should succeed");
        assert!(logger.borrow(py).log(FemtoLevel::Info, "ok").is_some());
        assert!(logger.borrow(py).log(FemtoLevel::Error, "nope").is_none());
    });
}
