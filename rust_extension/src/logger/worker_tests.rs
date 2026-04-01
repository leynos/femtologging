//! Focused unit tests for logger worker helpers.

use std::any::Any;
use std::sync::Arc;

use log::Level;
use rstest::rstest;

use super::logger_tests_helpers::collecting_handler;
use super::*;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};

#[derive(Default)]
struct FailingHandler;

impl FemtoHandlerTrait for FailingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Err(HandlerError::Message("boom".to_string()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[rstest]
fn handle_log_record_continues_after_handler_errors() {
    let collecting_handler = collecting_handler();
    let failing = Arc::new(FailingHandler) as Arc<dyn FemtoHandlerTrait>;
    let collecting = collecting_handler.clone() as Arc<dyn FemtoHandlerTrait>;

    FemtoLogger::handle_log_record(QueuedRecord {
        record: FemtoLogRecord::new("worker", FemtoLevel::Info, "survives"),
        handlers: vec![failing, collecting],
    });

    let collected = collecting_handler.collected();
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].message(), "survives");
}

#[rstest]
#[case::normal_exit(false)]
#[case::panic_exit(true)]
fn log_join_result_handles_worker_exit_paths(#[case] should_panic: bool) {
    if should_panic {
        install_test_logger();
    }
    let handle = std::thread::spawn(move || {
        assert!(!should_panic, "worker boom");
    });

    worker::log_join_result(handle);

    if should_panic {
        let captured = take_logged_messages();
        assert!(
            captured.iter().any(|record| record.level == Level::Warn
                && record.message.contains("worker thread panicked")),
            "expected a warning log when the worker thread panics"
        );
    }
}
