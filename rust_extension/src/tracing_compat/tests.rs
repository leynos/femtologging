//! Unit tests for the tracing compatibility bridge.

use std::sync::Arc;

use pyo3::Python;
use rstest::rstest;
use serial_test::serial;
use tracing_subscriber::prelude::*;

use super::{FALLBACK_EVENT_MESSAGE, FemtoTracingLayer};
use crate::FemtoLevel;
use crate::handler::FemtoHandlerTrait;
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;

fn attach_collecting_handler(logger_name: &str) -> Arc<CollectingHandler> {
    let handler = Arc::new(CollectingHandler::default());
    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        logger.borrow(py).clear_handlers();
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
    });
    handler
}

#[rstest]
#[case(tracing::Level::TRACE, FemtoLevel::Trace)]
#[case(tracing::Level::DEBUG, FemtoLevel::Debug)]
#[case(tracing::Level::INFO, FemtoLevel::Info)]
#[case(tracing::Level::WARN, FemtoLevel::Warn)]
#[case(tracing::Level::ERROR, FemtoLevel::Error)]
fn level_mapping_is_direct(#[case] level: tracing::Level, #[case] expected: FemtoLevel) {
    assert_eq!(FemtoTracingLayer::map_level(&level), expected);
}

#[rstest]
#[serial]
fn events_are_forwarded_with_structured_fields() {
    manager::reset_manager();
    let logger_name = "tracing.bridge.events";
    let handler = attach_collecting_handler(logger_name);
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            target: "tracing.bridge.events",
            answer = 42_u64,
            success = true,
            ratio = 1.5_f64,
            payload = ?vec!["a", "b"],
            "hello from tracing"
        );
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );
    });
    let records = handler.collected();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.logger(), logger_name);
    assert_eq!(record.level_str(), "INFO");
    assert_eq!(record.message(), "hello from tracing");
    assert_eq!(record.metadata().key_values["answer"], "42");
    assert_eq!(record.metadata().key_values["success"], "true");
    assert_eq!(record.metadata().key_values["ratio"], "1.5");
    assert_eq!(record.metadata().key_values["payload"], "[\"a\", \"b\"]");
}

#[rstest]
#[serial]
fn invalid_targets_fall_back_to_root() {
    manager::reset_manager();
    let handler = attach_collecting_handler("root");
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "invalid..target", request_id = "abc-123");
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, "root").expect("logger created");
        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );
    });
    let records = handler.collected();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].logger(), "root");
    assert_eq!(records[0].message(), "tracing event (request_id=abc-123)");
}

#[rstest]
#[serial]
fn event_without_message_uses_stable_fallback() {
    manager::reset_manager();
    let logger_name = "tracing.bridge.fallback";
    let handler = attach_collecting_handler(logger_name);
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "tracing.bridge.fallback", answer = 42_u64, success = true);
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );
    });
    let records = handler.collected();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].message(),
        "tracing event (answer=42, success=true)"
    );
}

#[rstest]
#[serial]
fn logger_thresholds_still_apply() {
    manager::reset_manager();
    let logger_name = "tracing.bridge.threshold";
    let handler = attach_collecting_handler(logger_name);
    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "tracing.bridge.threshold", "suppressed");
        tracing::warn!(target: "tracing.bridge.threshold", "visible");
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );
    });
    let records = handler.collected();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].message(), "visible");
    assert_eq!(records[0].level_str(), "WARN");
}

#[rstest]
#[serial]
fn femtologging_targets_are_ignored() {
    manager::reset_manager();
    let handler = attach_collecting_handler("root");
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "femtologging::internal", "ignored");
    });

    assert!(
        handler.collected().is_empty(),
        "internal femtologging targets must not recurse back through the layer"
    );
}

#[rstest]
#[serial]
fn active_span_fields_are_merged_into_event_metadata() {
    manager::reset_manager();
    let logger_name = "tracing.bridge.span";
    let handler = attach_collecting_handler(logger_name);
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        let outer =
            tracing::info_span!(target: "tracing.bridge.span", "request", request_id = "req-42");
        let _outer_guard = outer.enter();
        let inner = tracing::info_span!(target: "tracing.bridge.span", "step", attempt = 2_u64);
        let _inner_guard = inner.enter();
        tracing::info!(target: "tracing.bridge.span", success = true, "inside span");
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );
    });
    let records = handler.collected();
    assert_eq!(records.len(), 1);
    let key_values = &records[0].metadata().key_values;
    assert_eq!(key_values["span.0.name"], "request");
    assert_eq!(key_values["span.0.request_id"], "req-42");
    assert_eq!(key_values["span.1.name"], "step");
    assert_eq!(key_values["span.1.attempt"], "2");
    assert_eq!(key_values["success"], "true");
}

#[rstest]
#[serial]
fn events_outside_spans_do_not_gain_span_metadata() {
    manager::reset_manager();
    let logger_name = "tracing.bridge.outside";
    let handler = attach_collecting_handler(logger_name);
    let subscriber = tracing_subscriber::registry().with(FemtoTracingLayer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "tracing.bridge.outside", "outside span");
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, logger_name).expect("logger created");
        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );
    });
    let records = handler.collected();
    assert_eq!(records.len(), 1);
    assert!(
        records[0]
            .metadata()
            .key_values
            .keys()
            .all(|key| !key.starts_with("span.")),
        "events outside spans must not carry span-prefixed metadata"
    );
}

#[rstest]
fn fallback_message_constant_is_human_readable() {
    assert_eq!(FALLBACK_EVENT_MESSAGE, "tracing event");
}
