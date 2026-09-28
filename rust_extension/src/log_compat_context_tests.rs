//! Regression coverage for Rust scoped context in the `log` bridge.

use super::FemtoLogAdapter;
use crate::handler::FemtoHandlerTrait;
use crate::log_context;
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;
use log::{LevelFilter, Log};
use pyo3::prelude::*;
use rstest::{fixture, rstest};
use std::sync::{
    Arc, Once,
    atomic::{AtomicUsize, Ordering},
};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[fixture]
fn unique_logger_name() -> String {
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("bridge.context.{suffix}")
}

#[fixture]
fn log_max_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| log::set_max_level(LevelFilter::Trace));
}

#[rstest]
fn adapter_merges_active_rust_context(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger.borrow(py).add_handler(handler.clone());

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
        assert_eq!(records[0].metadata().key_values["request_id"], "req-42");
    });
}
