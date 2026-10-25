//! Isolated high-cardinality tests for the Rust `log` bridge.

use pyo3::prelude::*;
use std::fmt::Arguments;
use std::sync::Arc;

use super::{install_global_logger, resolve_logger};
use crate::handler::FemtoHandlerTrait;
use crate::logger::worker_count_for_test;
use crate::manager::{self, MAX_LOGGERS, named_logger_count_for_test};
use crate::test_utils::collecting_handler::CollectingHandler;

fn log_target_record(target: &str, level: log::Level, args: Arguments<'_>) {
    let record = log::Record::builder()
        .args(args)
        .level(level)
        .target(target)
        .build();
    log::logger().log(&record);
}

fn log_valid_targets() {
    for index in 0..MAX_LOGGERS {
        let target = format!("log-bridge-{index}");
        log_target_record(
            &target,
            log::Level::Debug,
            format_args!("lookup target {index}"),
        );
    }
}

fn add_collecting_handler(py: Python<'_>, root: &Py<crate::FemtoLogger>) -> Arc<CollectingHandler> {
    let handler = Arc::new(CollectingHandler::default());
    root.borrow(py)
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");
    handler
}

fn log_overflow_targets(py: Python<'_>, root: &Py<crate::FemtoLogger>) {
    for index in 0..3 {
        let target = format!("log-bridge-overflow-{index}");
        log_target_record(
            &target,
            log::Level::Info,
            format_args!("overflow record {index}"),
        );

        let (resolved_name, resolved) =
            resolve_logger(py, &target).expect("a refused valid target should resolve to root");
        assert_eq!(resolved_name, "root");
        assert!(resolved.is(root), "fallback must preserve root identity");
    }
}

fn assert_root_received_overflow(
    py: Python<'_>,
    root: &Py<crate::FemtoLogger>,
    handler: &CollectingHandler,
) {
    assert!(
        root.borrow(py).flush_handlers(),
        "root should drain its queue"
    );
    let collected_names = handler
        .collected()
        .iter()
        .map(|record| record.logger().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        collected_names,
        vec!["root"; 3],
        "records past the cap must reach the existing root logger",
    );
}

fn assert_logger_identity(py: Python<'_>, name: &str, expected: &Py<crate::FemtoLogger>) {
    assert!(
        manager::get_logger(py, name)
            .expect("existing logger still resolves")
            .is(expected),
        "logger identity must survive saturation",
    );
}

fn assert_bounded_registry_and_workers(initial_worker_count: usize) {
    assert_eq!(named_logger_count_for_test(), MAX_LOGGERS);
    assert_eq!(worker_count_for_test(), initial_worker_count);
}

fn run_isolated_child(test_filter: &str) {
    let output = crate::test_utils::isolated_child::run_isolated_test(test_filter)
        .expect("the isolated bridge test should start");
    assert!(
        output.status.success(),
        "isolated test {test_filter:?} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn the_log_bridge_keeps_valid_target_cardinality_bounded() {
    run_isolated_child("log_bridge_saturation_child");
}

#[test]
#[ignore = "run in a fresh process because the manager is process-global"]
fn log_bridge_saturation_child() {
    assert!(
        install_global_logger(),
        "the child should own the global logger"
    );

    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root should be available");
        let initial_worker_count = worker_count_for_test();
        assert!(initial_worker_count > 0, "resolving root starts the pool");
        log_valid_targets();

        assert_eq!(
            named_logger_count_for_test(),
            MAX_LOGGERS,
            "valid bridge targets should fill, but not exceed, the cap",
        );
        assert_eq!(
            worker_count_for_test(),
            initial_worker_count,
            "dispatch worker count must not track target count",
        );

        let first = manager::get_logger(py, "log-bridge-0").expect("first logger remains");
        let handler = add_collecting_handler(py, &root);
        log_overflow_targets(py, &root);
        assert_root_received_overflow(py, &root, &handler);
        assert_logger_identity(py, "log-bridge-0", &first);
        assert_logger_identity(py, "root", &root);
        assert_bounded_registry_and_workers(initial_worker_count);
    });
}
