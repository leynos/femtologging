//! Isolated high-cardinality tests for tracing target resolution.

use pyo3::prelude::*;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use tracing::callsite::{Callsite, Identifier};
use tracing::field::FieldSet;
use tracing::{Dispatch, Event, Level, Metadata, metadata::Kind};
use tracing_subscriber::prelude::*;

use super::FemtoTracingLayer;
use crate::handler::FemtoHandlerTrait;
use crate::logger::worker_count_for_test;
use crate::manager::{self, MAX_LOGGERS, named_logger_count_for_test};
use crate::test_utils::collecting_handler::CollectingHandler;

/// Callsite metadata used to emit an event with a runtime-selected target.
struct DynamicTargetCallsite {
    /// Metadata associated with this unique callsite.
    metadata: OnceLock<&'static Metadata<'static>>,
}

impl Callsite for DynamicTargetCallsite {
    fn set_interest(&self, _interest: tracing::subscriber::Interest) {}

    fn metadata(&self) -> &Metadata<'_> {
        self.metadata
            .get()
            .expect("callsite metadata should be initialized before dispatch")
    }
}

/// Build static tracing metadata around a caller-selected target.
fn metadata_for_target(target: String) -> &'static Metadata<'static> {
    // Tracing metadata is static by contract. This helper runs in a short-lived
    // child process, so leaking one target and callsite per event is bounded by
    // this test process and reclaimed when it exits.
    let target = Box::leak(target.into_boxed_str());
    let callsite = Box::leak(Box::new(DynamicTargetCallsite {
        metadata: OnceLock::new(),
    }));
    let fields = FieldSet::new(&[], Identifier(callsite));
    let metadata = Box::leak(Box::new(Metadata::new(
        "cardinality_event",
        target,
        Level::INFO,
        None,
        None,
        None,
        fields,
        Kind::EVENT,
    )));
    assert!(
        callsite.metadata.set(metadata).is_ok(),
        "each dynamic callsite receives metadata once",
    );
    metadata
}

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
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);
    let dispatch = Dispatch::new(subscriber);

    let (root, handler, initial_worker_count) = Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root should be available");
        let handler = Arc::new(CollectingHandler::default());
        root.borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
            .expect("native collecting handler should register");
        let initial_worker_count = worker_count_for_test();
        assert!(initial_worker_count > 0, "resolving root starts the pool");
        (root, handler, initial_worker_count)
    });

    for index in 0..MAX_LOGGERS {
        let target = format!("tracing-bridge-{index}");
        let metadata = metadata_for_target(target.clone());
        assert!(
            dispatch.enabled(metadata),
            "the subscriber should enable valid target {target:?}",
        );

        // The event still enters the layer's `on_event` path, but does not
        // also enqueue a second copy at root while this test fills the cap.
        Python::attach(|py| {
            manager::get_logger(py, &target)
                .expect("the layer's enabled path should create the logger")
                .borrow(py)
                .set_propagate(false);
        });
        let values = metadata.fields().value_set_all(&[]);
        dispatch.event(&Event::new(metadata, &values));
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

    let first = Python::attach(|py| {
        manager::get_logger(py, "tracing-bridge-0").expect("first logger remains")
    });
    for index in 0..3 {
        let target = format!("tracing-bridge-overflow-{index}");
        let metadata = metadata_for_target(target.clone());
        assert!(
            dispatch.enabled(metadata),
            "an overflow target should stay enabled through root fallback",
        );
        let values = metadata.fields().value_set_all(&[]);
        dispatch.event(&Event::new(metadata, &values));
    }

    Python::attach(|py| {
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
            "events beyond the cap must reach the existing root logger",
        );

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
