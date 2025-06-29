//! Tests for `FemtoFileHandler`.
//!
//! These cover single-record writes, multi-record writes, queue overflow
//! handling and concurrent usage from multiple threads.

use std::fs;
use std::sync::Arc;
use std::thread;

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord};
use tempfile::NamedTempFile;

/// Execute `f` with a `FemtoFileHandler` backed by a fresh temporary file
/// and return whatever the handler wrote.
///
/// `capacity` is forwarded to `FemtoFileHandler::with_capacity`.
pub fn with_temp_file_handler<F>(capacity: usize, f: F) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    let tmp = NamedTempFile::new().expect("failed to create temp file");
    let path = tmp.path().to_path_buf();
    {
        let handler = FemtoFileHandler::with_capacity(&path, DefaultFormatter, capacity)
            .expect("failed to create file handler");
        f(&handler);
    }
    fs::read_to_string(&path).expect("failed to read log output")
}

#[test]
fn file_handler_writes_to_file() {
    let output = with_temp_file_handler(10, |h| {
        h.handle(FemtoLogRecord::new("core", "INFO", "hello"));
    });

    assert_eq!(output, "core [INFO] hello\n");
}

#[test]
fn multiple_records_are_serialised() {
    let output = with_temp_file_handler(10, |h| {
        h.handle(FemtoLogRecord::new("core", "INFO", "first"));
        h.handle(FemtoLogRecord::new("core", "WARN", "second"));
        h.handle(FemtoLogRecord::new("core", "ERROR", "third"));
    });

    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n",
    );
}

#[test]
fn queue_overflow_drops_excess_records() {
    let output = with_temp_file_handler(3, |h| {
        h.handle(FemtoLogRecord::new("core", "INFO", "first"));
        h.handle(FemtoLogRecord::new("core", "WARN", "second"));
        h.handle(FemtoLogRecord::new("core", "ERROR", "third"));
        h.handle(FemtoLogRecord::new("core", "DEBUG", "fourth"));
        h.handle(FemtoLogRecord::new("core", "TRACE", "fifth"));
    });

    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n",
    );
}

#[test]
fn file_handler_concurrent_usage() {
    let tmp = NamedTempFile::new().expect("failed to create temp file");
    let path = tmp.path().to_path_buf();
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
    let output = fs::read_to_string(&path).expect("failed to read log output");
    for i in 0..10 {
        assert!(output.contains(&format!("core [INFO] msg{}", i)));
    }
}
#[test]
fn file_handler_open_failure() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let path = dir.path().join("missing").join("file.log");
    assert!(FemtoFileHandler::new(&path).is_err());
}
