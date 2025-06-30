//! Tests for `FemtoFileHandler`.
//!
//! These cover single-record writes, multi-record writes, queue overflow
//! handling and concurrent usage from multiple threads.

use std::fs;
use std::sync::Arc;
use std::thread;

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord, OverflowPolicy,
};
use tempfile::NamedTempFile;

/// Execute `f` with a `FemtoFileHandler` backed by a fresh temporary file
/// and return whatever the handler wrote.
///
/// `capacity` is forwarded to `FemtoFileHandler::with_capacity`.
fn with_temp_file_handler_generic<F>(
    capacity: usize,
    flush_interval: usize,
    policy: OverflowPolicy,
    f: F,
) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    let tmp = NamedTempFile::new().expect("failed to create temp file");
    let path = tmp.path().to_path_buf();
    {
        let handler = FemtoFileHandler::with_capacity_policy(
            &path,
            DefaultFormatter,
            capacity,
            flush_interval,
            policy,
        )
        .expect("failed to create file handler");
        f(&handler);
    }
    fs::read_to_string(&path).expect("failed to read log output")
}

pub fn with_temp_file_handler<F>(capacity: usize, f: F) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    with_temp_file_handler_generic(capacity, 1, OverflowPolicy::Drop, f)
}

pub fn with_temp_file_handler_flush<F>(capacity: usize, flush_interval: usize, f: F) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    with_temp_file_handler_generic(capacity, flush_interval, OverflowPolicy::Drop, f)
}

fn with_temp_file_handler_blocking<F>(capacity: usize, f: F) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    with_temp_file_handler_generic(capacity, 1, OverflowPolicy::Block, f)
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
fn blocking_policy_persists_records() {
    let output = with_temp_file_handler_blocking(1, |h| {
        for i in 0..5 {
            h.handle(FemtoLogRecord::new("core", "INFO", &format!("msg{}", i)));
        }
    });

    for i in 0..5 {
        assert!(output.contains(&format!("core [INFO] msg{}", i)));
    }
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

#[test]
fn file_handler_custom_flush_interval() {
    let output = with_temp_file_handler_flush(8, 2, |h| {
        h.handle(FemtoLogRecord::new("core", "INFO", "first"));
        h.handle(FemtoLogRecord::new("core", "INFO", "second"));
        h.handle(FemtoLogRecord::new("core", "INFO", "third"));
    });

    assert_eq!(
        output,
        "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n",
    );
}

#[test]
fn file_handler_flush_interval_zero() {
    let output = with_temp_file_handler_flush(8, 0, |h| {
        h.handle(FemtoLogRecord::new("core", "INFO", "message"));
    });
    assert_eq!(output, "core [INFO] message\n");
}

#[test]
fn file_handler_flush_interval_one() {
    let output = with_temp_file_handler_flush(8, 1, |h| {
        h.handle(FemtoLogRecord::new("core", "INFO", "message"));
    });
    assert_eq!(output, "core [INFO] message\n");
}
