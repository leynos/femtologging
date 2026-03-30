//! Focused unit tests for logger worker helpers.

use std::any::Any;
use std::sync::Arc;

use rstest::{fixture, rstest};

use super::*;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::test_utils::collecting_handler::CollectingHandler;

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

#[fixture]
fn collecting_handler() -> Arc<CollectingHandler> {
    Arc::new(CollectingHandler::new())
}

#[rstest]
fn handle_log_record_continues_after_handler_errors(collecting_handler: Arc<CollectingHandler>) {
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
    let handle = std::thread::spawn(move || {
        assert!(!should_panic, "worker boom");
    });

    worker::log_join_result(handle);
}
