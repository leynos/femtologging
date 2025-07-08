//! Tests for `FemtoFileHandler`.
//!
//! These cover single-record writes, multi-record writes, queue overflow
//! handling and concurrent usage from multiple threads.

use std::fs;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord, OverflowPolicy,
    TestConfig,
};
use tempfile::NamedTempFile;

#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

/// Execute `f` with a `FemtoFileHandler` backed by a fresh temporary file
/// and return whatever the handler wrote.
///
/// `capacity` is forwarded to `FemtoFileHandler::with_capacity`.
fn with_temp_file_handler_generic<F>(capacity: usize, flush_interval: usize, f: F) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    let tmp = NamedTempFile::new().expect("failed to create temp file");
    let path = tmp.path().to_path_buf();
    {
        let handler = FemtoFileHandler::with_capacity_flush_interval(
            &path,
            DefaultFormatter,
            capacity,
            flush_interval,
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
    with_temp_file_handler_generic(capacity, 1, f)
}

pub fn with_temp_file_handler_flush<F>(capacity: usize, flush_interval: usize, f: F) -> String
where
    F: FnOnce(&FemtoFileHandler),
{
    with_temp_file_handler_generic(capacity, flush_interval, f)
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
        for i in 0..10 {
            h.handle(FemtoLogRecord::new("core", "INFO", &format!("msg{i}")));
        }
    });

    assert_eq!(
        output,
        "core [INFO] msg0\ncore [INFO] msg1\ncore [INFO] msg2\n",
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

#[test]
fn blocking_policy_waits_for_space() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let start = Arc::new(Barrier::new(2));
    let mut cfg = TestConfig::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    cfg.overflow_policy = OverflowPolicy::Block;
    cfg.start_barrier = Some(Arc::clone(&start));
    let handler = Arc::new(FemtoFileHandler::with_writer_for_test(cfg));

    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    let h = Arc::clone(&handler);
    let t = thread::spawn(move || {
        h.handle(FemtoLogRecord::new("core", "INFO", "second"));
    });
    thread::sleep(Duration::from_millis(50));
    assert!(!t.is_finished());
    start.wait();
    t.join().unwrap();
    assert!(handler.flush());

    let buf = buffer.lock().unwrap();
    let output = String::from_utf8_lossy(&buf);
    assert!(output.contains("core [INFO] first"));
    assert!(output.contains("core [INFO] second"));
    let first_idx = output.find("core [INFO] first").unwrap();
    let second_idx = output.find("core [INFO] second").unwrap();
    assert!(first_idx < second_idx);
}

#[test]
fn timeout_policy_gives_up() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let start = Arc::new(Barrier::new(2));
    let mut cfg = TestConfig::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    cfg.overflow_policy = OverflowPolicy::Timeout(Duration::from_millis(50));
    cfg.start_barrier = Some(Arc::clone(&start));
    let handler = FemtoFileHandler::with_writer_for_test(cfg);

    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    let start_time = Instant::now();
    handler.handle(FemtoLogRecord::new("core", "INFO", "second"));
    assert!(start_time.elapsed() >= Duration::from_millis(50));
    start.wait();
}
