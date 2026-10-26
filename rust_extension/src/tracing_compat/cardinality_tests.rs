//! Isolated high-cardinality tests for tracing target resolution.

use pyo3::prelude::*;
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

fn emit_target(dispatch: &Dispatch, target: &str, disable_propagation: bool) {
    let metadata = metadata_for_target(target.to_owned());
    assert!(
        dispatch.enabled(metadata),
        "the subscriber should enable target {target:?}",
    );
    if disable_propagation {
        Python::attach(|py| {
            manager::get_logger(py, target)
                .expect("the enabled path should create the logger")
                .borrow(py)
                .set_propagate(false);
        });
    }
    let values = metadata.fields().value_set_all(&[]);
    dispatch.event(&Event::new(metadata, &values));
}

fn dispatch_valid_targets(dispatch: &Dispatch) {
    for index in 0..MAX_LOGGERS {
        emit_target(dispatch, &format!("tracing-bridge-{index}"), true);
    }
}

fn dispatch_overflow_targets(dispatch: &Dispatch) {
    for index in 0..3 {
        emit_target(dispatch, &format!("tracing-bridge-overflow-{index}"), false);
    }
}

fn create_root_logger() -> (Py<crate::FemtoLogger>, Arc<CollectingHandler>, usize) {
    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root should be available");
        let handler = Arc::new(CollectingHandler::default());
        root.borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
            .expect("native collecting handler should register");
        let initial_worker_count = worker_count_for_test();
        assert!(initial_worker_count > 0, "resolving root starts the pool");
        (root, handler, initial_worker_count)
    })
}

fn assert_saturated_state(
    root: &Py<crate::FemtoLogger>,
    first: &Py<crate::FemtoLogger>,
    handler: &CollectingHandler,
    initial_worker_count: usize,
) {
    Python::attach(|py| {
        assert!(
            root.borrow(py).flush_handlers(),
            "root should drain its queue",
        );
        let collected_names = handler
            .collected()
            .iter()
            .map(|record| record.logger().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            collected_names,
            vec!["root"; 3],
            "events beyond the cap must reach the existing root logger",
        );
        assert!(
            manager::get_logger(py, "tracing-bridge-0")
                .expect("existing logger still resolves")
                .is(first),
            "existing logger identity must survive saturation",
        );
        assert!(
            manager::get_logger(py, "root")
                .expect("root remains available")
                .is(root),
            "root identity must survive saturation",
        );
        assert_eq!(named_logger_count_for_test(), MAX_LOGGERS);
        assert_eq!(worker_count_for_test(), initial_worker_count);
    });
}

fn run_isolated_child(test_filter: &str) {
    let output = crate::test_utils::isolated_child::run_isolated_test(test_filter)
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
    let (root, handler, initial_worker_count) = create_root_logger();

    dispatch_valid_targets(&dispatch);

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
    dispatch_overflow_targets(&dispatch);
    assert_saturated_state(&root, &first, &handler, initial_worker_count);
}
