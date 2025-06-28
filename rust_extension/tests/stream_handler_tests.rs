use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use _femtologging_rs::{DefaultFormatter, FemtoHandlerTrait, FemtoLogRecord, FemtoStreamHandler};
use rstest::rstest;

#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

fn make_handler() -> (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler) {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = FemtoStreamHandler::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);
    (buffer, handler)
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
}

#[rstest]
fn stream_handler_writes_to_buffer() {
    let (buffer, handler) = make_handler();
    handler.handle(FemtoLogRecord::new("core", "INFO", "hello"));
    drop(handler); // ensure thread completes

    assert_eq!(read_output(&buffer), "core: INFO - hello\n");
}

#[rstest]
fn stream_handler_multiple_records() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = FemtoStreamHandler::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "WARN", "second"));
    handler.handle(FemtoLogRecord::new("core", "ERROR", "third"));
    drop(handler);

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert_eq!(
        output,
        "core: INFO - first\ncore: WARN - second\ncore: ERROR - third\n"
    );
}

#[rstest]
fn stream_handler_concurrent_usage() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(FemtoStreamHandler::new(
        SharedBuf(Arc::clone(&buffer)),
        DefaultFormatter,
    ));

    let mut handles = vec![];
    for i in 0..10 {
        let h = Arc::clone(&handler);
        handles.push(thread::spawn(move || {
            h.handle(FemtoLogRecord::new("core", "INFO", &format!("msg{}", i)));
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    drop(handler);

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    for i in 0..10 {
        assert!(output.contains(&format!("core: INFO - msg{}", i)));
    }
}

#[rstest]
fn stream_handler_trait_object_usage() {
    let (buffer, handler) = make_handler();
    let handler: Box<dyn FemtoHandlerTrait> = Box::new(handler);
    handler.handle(FemtoLogRecord::new("core", "INFO", "trait"));
    drop(handler);

    assert_eq!(read_output(&buffer), "core: INFO - trait\n");
}

#[rstest]
fn stream_handler_poisoned_mutex() {
    // Poison the mutex by panicking while holding the lock
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let test_buffer = Arc::clone(&buffer);
    {
        let b = Arc::clone(&buffer);
        let _ = std::panic::catch_unwind(move || {
            let _guard = b.lock().unwrap();
            panic!("poison");
        });
    }

    let handler = FemtoStreamHandler::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);
    handler.handle(FemtoLogRecord::new("core", "INFO", "ok"));
    drop(handler);

    // The buffer should remain poisoned; handler must not panic
    assert!(
        test_buffer.lock().is_err(),
        "Buffer mutex should remain poisoned"
    );
}
