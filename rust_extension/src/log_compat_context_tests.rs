//! Regression coverage for Rust scoped context in the `log` bridge.

use super::FemtoLogAdapter;
use crate::handler::FemtoHandlerTrait;
use crate::log_context;
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;
use crate::test_utils::logging_test_helpers::{log_max_level, unique_logger_name};
use log::Log;
use pyo3::prelude::*;
use rstest::rstest;
use std::sync::Arc;

#[rstest]
fn adapter_merges_active_rust_context(unique_logger_name: String) {
    log_max_level();
    let adapter = FemtoLogAdapter;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");

        let _guard = log_context::push_log_context([("request_id", "req-42")])
            .expect("context push should succeed");
        let record = log::Record::builder()
            .args(format_args!("bridge"))
            .level(log::Level::Info)
            .target(&unique_logger_name)
            .build();

        adapter.log(&record);
        assert!(logger.borrow(py).flush_handlers());

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        let collected = records
            .first()
            .expect("the bridge should collect exactly one record");
        assert_eq!(
            collected
                .metadata()
                .key_values
                .get("request_id")
                .map(String::as_str),
            Some("req-42")
        );
    });
}
