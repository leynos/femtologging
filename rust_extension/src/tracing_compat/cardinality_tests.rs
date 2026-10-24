//! Isolated high-cardinality tests for tracing target resolution.

use pyo3::prelude::*;
use std::process::Command;

use super::FemtoTracingLayer;
use crate::logger::worker_count_for_test;
use crate::manager::{self, MAX_LOGGERS, named_logger_count_for_test};

fn run_isolated_child(test_filter: &str) {
    let executable = std::env::current_exe().expect("the Rust test executable should be known");
    let output = Command::new(executable)
        .env_clear()
        .args(["--ignored", test_filter, "--nocapture"])
        .output()
        .expect("the isolated tracing test should start");
    assert!(
        output.status.success(),
        "isolated test {test_filter:?} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn tracing_target_resolution_keeps_cardinality_bounded() {
    run_isolated_child("tracing_target_saturation_child");
}

#[test]
#[ignore = "run in a fresh process because the manager is process-global"]
fn tracing_target_saturation_child() {
    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root should be available");
        let initial_worker_count = worker_count_for_test();
        assert!(initial_worker_count > 0, "resolving root starts the pool");

        for index in 0..MAX_LOGGERS {
            let target = format!("tracing-bridge-{index}");
            let (resolved_name, _logger) = FemtoTracingLayer::resolve_logger(py, &target)
                .expect("a valid tracing target should resolve");
            assert_eq!(resolved_name.as_ref(), target);
        }

        assert_eq!(
            named_logger_count_for_test(),
            MAX_LOGGERS,
            "valid tracing targets should fill, but not exceed, the cap",
        );
        assert_eq!(
            worker_count_for_test(),
            initial_worker_count,
            "dispatch worker count must not track target count",
        );

        let first = manager::get_logger(py, "tracing-bridge-0").expect("first logger remains");
        for index in 0..3 {
            let target = format!("tracing-bridge-overflow-{index}");
            let (resolved_name, resolved) = FemtoTracingLayer::resolve_logger(py, &target)
                .expect("a refused valid target should resolve to root");
            assert_eq!(resolved_name.as_ref(), "root");
            assert!(resolved.is(&root), "fallback must preserve root identity");
        }

        assert!(
            manager::get_logger(py, "tracing-bridge-0")
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
