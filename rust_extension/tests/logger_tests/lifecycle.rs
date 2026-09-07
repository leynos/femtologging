//! Logger shutdown and worker-lifecycle behavioural tests.

use super::{HandlerTuple, handler_tuple, read_output};
use _femtologging_rs::{FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, FemtoLogger, QueuedRecord};
use rstest::rstest;
use std::sync::Arc;

#[test]
fn drop_with_sender_clone_exits() {
    let logger = FemtoLogger::new("clone".to_string());
    let tx = logger.clone_sender_for_test().expect("sender should exist");
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let thread_barrier = Arc::clone(&barrier);
    let thread = std::thread::spawn(move || {
        thread_barrier.wait();
        let result = tx.send(QueuedRecord {
            record: FemtoLogRecord::new("clone", FemtoLevel::Info, "late"),
            handlers: Vec::new(),
            #[cfg(feature = "python")]
            context: None,
        });
        assert!(
            result.is_err(),
            "expected send to fail after logger is dropped"
        );
    });
    drop(logger);
    barrier.wait();
    thread.join().expect("worker thread panicked");
}

#[rstest]
fn logger_drains_records_on_drop(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let handler = Arc::new(handler);
    let logger = FemtoLogger::new("core".to_string());
    logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
    logger.log(FemtoLevel::Info, "one");
    logger.log(FemtoLevel::Info, "two");
    logger.log(FemtoLevel::Info, "three");
    drop(logger);
    drop(handler);
    assert_eq!(
        read_output(&buffer),
        "core [INFO] one\ncore [INFO] two\ncore [INFO] three\n"
    );
}
