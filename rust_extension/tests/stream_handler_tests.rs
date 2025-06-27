use std::sync::{Arc, Mutex};

use _femtologging_rs::{DefaultFormatter, FemtoHandler, FemtoLogRecord, FemtoStreamHandler};
use rstest::rstest;

#[rstest]
fn stream_handler_writes_to_buffer() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = FemtoStreamHandler::new(Arc::clone(&buffer), Arc::new(DefaultFormatter));
    handler.handle(FemtoLogRecord::new("core", "INFO", "hello"));
    drop(handler); // ensure thread completes

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert_eq!(output, "core: INFO - hello\n");
}
