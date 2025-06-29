//! Tests for `FemtoFileHandler`.
//!
//! These cover single-record writes, multi-record writes, queue overflow
//! handling and concurrent usage from multiple threads.

use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::thread;

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord};
use rstest::*;
use tempfile::NamedTempFile;

#[fixture]
fn temp_log_file() -> NamedTempFile {
    NamedTempFile::new().expect("Failed to create temporary file")
}

fn read_file(path: &std::path::Path) -> String {
    let mut contents = String::new();
    File::open(path)
        .expect("Failed to open test file")
        .read_to_string(&mut contents)
        .expect("Failed to read test file contents");
    contents
}

#[rstest]
fn file_handler_writes_to_file(mut temp_log_file: NamedTempFile) {
    let path = temp_log_file.path().to_path_buf();
    let handler = FemtoFileHandler::new(&path).expect("Failed to create file handler");
    handler.handle(FemtoLogRecord::new("core", "INFO", "hello"));
    drop(handler);
    assert_eq!(read_file(&path), "core [INFO] hello\n");
}

#[rstest]
fn file_handler_multiple_records(mut temp_log_file: NamedTempFile) {
    let path = temp_log_file.path().to_path_buf();
    let handler = FemtoFileHandler::with_capacity(&path, DefaultFormatter, 10)
        .expect("Failed to create file handler");
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "WARN", "second"));
    handler.handle(FemtoLogRecord::new("core", "ERROR", "third"));
    drop(handler);
    let output = read_file(&path);
    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    );
}

#[rstest]
fn file_handler_queue_overflow_drops_excess_records(mut temp_log_file: NamedTempFile) {
    let path = temp_log_file.path().to_path_buf();
    let handler = FemtoFileHandler::with_capacity(&path, DefaultFormatter, 3)
        .expect("Failed to create file handler");
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "WARN", "second"));
    handler.handle(FemtoLogRecord::new("core", "ERROR", "third"));
    handler.handle(FemtoLogRecord::new("core", "DEBUG", "fourth"));
    handler.handle(FemtoLogRecord::new("core", "TRACE", "fifth"));
    drop(handler);
    let output = read_file(&path);
    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    );
}

#[rstest]
fn file_handler_concurrent_usage(mut temp_log_file: NamedTempFile) {
    let path = temp_log_file.path().to_path_buf();
    let handler = Arc::new(FemtoFileHandler::new(&path).expect("Failed to create file handler"));
    let mut handles = vec![];
    for i in 0..10 {
        let h = Arc::clone(&handler);
        handles.push(thread::spawn(move || {
            h.handle(FemtoLogRecord::new("core", "INFO", &format!("msg{}", i)));
        }));
    }
    for h in handles {
        h.join().expect("Thread panicked");
    }
    drop(handler);
    let output = read_file(&path);
    for i in 0..10 {
        assert!(output.contains(&format!("core [INFO] msg{}", i)));
    }
}
#[rstest]
fn file_handler_open_failure() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let path = dir.path().join("missing").join("file.log");
    assert!(FemtoFileHandler::new(&path).is_err());
}
