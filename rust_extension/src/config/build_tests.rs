//! Unit tests for configuration runtime component construction.

use super::test_utils::gil_and_clean_manager;
use super::{ConfigBuilder, ConfigError, LoggerConfigBuilder};
use crate::StreamHandlerBuilder;
use crate::handlers::HandlerBuildError;
use rstest::rstest;
use serial_test::serial;

fn root_with_handler() -> LoggerConfigBuilder {
    LoggerConfigBuilder::new().with_handlers(["h"])
}

#[rstest]
#[serial]
fn unknown_handler_filter_id_rejected(_gil_and_clean_manager: ()) {
    let builder = ConfigBuilder::new()
        .with_handler(
            "h",
            StreamHandlerBuilder::stderr().with_filters(["missing"]),
        )
        .with_root_logger(root_with_handler());
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for an unknown handler filter id");
    assert!(matches!(err, ConfigError::UnknownIds(ids) if ids == vec!["missing".to_string()]));
}

#[rstest]
#[serial]
fn handler_build_failure_preserves_error_mapping(_gil_and_clean_manager: ()) {
    let builder = ConfigBuilder::new()
        .with_handler("h", StreamHandlerBuilder::stderr().with_capacity(0))
        .with_root_logger(root_with_handler());
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should preserve handler build failures");
    assert!(matches!(
        err,
        ConfigError::HandlerBuild {
            id,
            source: HandlerBuildError::InvalidConfig(message),
        } if id == "h" && message == "capacity must be greater than zero"
    ));
}

#[rstest]
#[serial]
fn formatter_backed_handler_build_failure_preserves_error_mapping(_gil_and_clean_manager: ()) {
    let builder = ConfigBuilder::new()
        .with_handler(
            "h",
            StreamHandlerBuilder::stderr().with_formatter("missing-formatter"),
        )
        .with_root_logger(root_with_handler());
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should preserve formatter resolution failures");
    assert!(matches!(
        err,
        ConfigError::HandlerBuild {
            id,
            source: HandlerBuildError::InvalidConfig(message),
        } if id == "h" && message == "unknown formatter id: missing-formatter"
    ));
}
