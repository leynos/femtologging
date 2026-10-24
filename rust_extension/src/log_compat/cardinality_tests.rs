//! Isolated high-cardinality tests for the Rust `log` bridge.

use pyo3::prelude::*;
use std::process::Command;
use std::sync::Arc;

use super::{install_global_logger, resolve_logger};
use crate::handler::FemtoHandlerTrait;
use crate::logger::worker_count_for_test;
use crate::manager::{self, MAX_LOGGERS, named_logger_count_for_test};
use crate::test_utils::collecting_handler::CollectingHandler;

fn run_isolated_child(test_filter: &str) {
    let executable = std::env::current_exe().expect("the Rust test executable should be known");
    let output = Command::new(executable)
        .env_clear()
        .args(["--ignored", test_filter, "--nocapture"])
        .output()
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

        for index in 0..MAX_LOGGERS {
            let target = format!("log-bridge-{index}");
            let args = format_args!("lookup target {index}");
            let record = log::Record::builder()
                .args(args)
                .level(log::Level::Debug)
                .target(&target)
                .build();
            log::logger().log(&record);
        }

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
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        root.borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");

        for index in 0..3 {
            let target = format!("log-bridge-overflow-{index}");
            let args = format_args!("overflow record {index}");
            let record = log::Record::builder()
                .args(args)
                .level(log::Level::Info)
                .target(&target)
                .build();
            log::logger().log(&record);

            let (resolved_name, resolved) =
                resolve_logger(py, &target).expect("a refused valid target should resolve to root");
            assert_eq!(resolved_name, "root");
            assert!(resolved.is(&root), "fallback must preserve root identity");
        }

        assert!(
            root.borrow(py).flush_handlers(),
            "root should drain its queue"
        );
        assert_eq!(
            handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected()
                .iter()
                .map(|record| record.logger().to_owned())
                .collect::<Vec<_>>(),
            vec!["root"; 3],
            "records past the cap must reach the existing root logger",
        );
        assert!(
            manager::get_logger(py, "log-bridge-0")
                .expect("existing logger still resolves")
                .is(&first),
            "existing logger identity must survive saturation",
        );
        assert!(
            manager::get_logger(py, "root")
                .expect("root remains available")
                .is(&root),
            "root identity must survive saturation",
        );
        assert_eq!(named_logger_count_for_test(), MAX_LOGGERS);
        assert_eq!(worker_count_for_test(), initial_worker_count);
    });
}
