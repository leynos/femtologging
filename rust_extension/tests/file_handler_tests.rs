//! Tests for `FemtoFileHandler`.
//!
//! These cover single-record writes, multi-record writes, queue overflow
//! handling and concurrent usage from multiple threads.

use std::fs;
use std::io;
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FemtoHandlerTrait, FemtoLevel, FemtoLogRecord,
    HandlerConfig, HandlerError, OverflowPolicy, TestConfig,
};
use tempfile::NamedTempFile;

#[path = "test_utils/handle_expect.rs"]
mod handle_expect;
#[path = "test_utils/shared_buffer.rs"]
mod shared_buffer;
use handle_expect::HandleExpect;
use shared_buffer::std::SharedBuf;
use shared_buffer::std::read_output;
use std::sync::{Arc, Mutex};

/// Execute `f` with a `FemtoFileHandler` backed by a fresh temporary file
/// and return whatever the handler wrote.
///
/// Setup failures are propagated rather than panicking, so the calling test
/// decides how to report them.
///
/// `capacity` is forwarded to `FemtoFileHandler::with_capacity_flush_policy`.
fn with_temp_file_handler_generic<F>(
    capacity: usize,
    flush_interval: usize,
    f: F,
) -> io::Result<String>
where
    F: FnOnce(&FemtoFileHandler),
{
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    {
        let cfg = HandlerConfig {
            capacity,
            flush_interval,
            overflow_policy: OverflowPolicy::Drop,
        };
        let handler = FemtoFileHandler::with_capacity_flush_policy(&path, DefaultFormatter, cfg)?;
        f(&handler);
    }
    fs::read_to_string(&path)
}

/// Run `f` against a temporary-file handler that flushes after every record.
fn with_temp_file_handler<F>(capacity: usize, f: F) -> io::Result<String>
where
    F: FnOnce(&FemtoFileHandler),
{
    with_temp_file_handler_generic(capacity, 1, f)
}

/// Run `f` against a temporary-file handler with an explicit flush interval.
fn with_temp_file_handler_flush<F>(
    capacity: usize,
    flush_interval: usize,
    f: F,
) -> io::Result<String>
where
    F: FnOnce(&FemtoFileHandler),
{
    with_temp_file_handler_generic(capacity, flush_interval, f)
}

/// A handler wired to an in-memory buffer whose worker thread is gated on a
/// barrier, so a test can saturate the queue before any record is drained.
///
/// This makes the overflow policies observable: without the barrier the worker
/// would drain records as fast as they are queued and the queue would never
/// fill.
struct OverflowHarness {
    buffer: Arc<Mutex<Vec<u8>>>,
    /// Released by the test to let the worker thread begin draining.
    start: Arc<Barrier>,
    handler: FemtoFileHandler,
}

impl OverflowHarness {
    /// Build a harness with the given queue `capacity` and overflow `policy`.
    fn new(capacity: usize, policy: OverflowPolicy) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let start = Arc::new(Barrier::new(2));
        let mut cfg = TestConfig::new(SharedBuf::new(Arc::clone(&buffer)), DefaultFormatter);
        cfg.capacity = capacity;
        cfg.flush_interval = 1;
        cfg.overflow_policy = policy;
        cfg.start_barrier = Some(Arc::clone(&start));
        let handler = FemtoFileHandler::with_writer_for_test(cfg);
        Self {
            buffer,
            start,
            handler,
        }
    }

    /// Return everything the worker thread has written so far.
    fn output(&self) -> Result<String, std::string::FromUtf8Error> {
        read_output(&self.buffer)
    }
}

#[test]
fn file_handler_writes_to_file() {
    let output = with_temp_file_handler(10, |h| {
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));
    })
    .expect("temporary file handler setup failed");

    assert_eq!(output, "core [INFO] hello\n");
}

#[test]
fn multiple_records_are_serialized() {
    let output = with_temp_file_handler(10, |h| {
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Warn, "second"));
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Error, "third"));
    })
    .expect("temporary file handler setup failed");

    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n",
    );
}

#[test]
fn queue_overflow_drops_excess_records() {
    let harness = OverflowHarness::new(3, OverflowPolicy::Drop);

    for i in 0..10 {
        let result = harness.handler.handle(FemtoLogRecord::new(
            "core",
            FemtoLevel::Info,
            &format!("msg{i}"),
        ));
        if i < 3 {
            assert!(result.is_ok(), "record {i} should fit in the queue");
        } else {
            assert_eq!(
                result,
                Err(HandlerError::QueueFull),
                "record {i} should be rejected when the queue is full",
            );
        }
    }
    // Allow the worker thread to start processing after all records are queued.
    harness.start.wait();
    let OverflowHarness {
        buffer, handler, ..
    } = harness;
    drop(handler);

    assert_eq!(
        read_output(&buffer).expect("buffer output should be valid UTF-8"),
        "core [INFO] msg0\ncore [INFO] msg1\ncore [INFO] msg2\n",
    );
}

#[test]
fn file_handler_concurrent_usage() {
    let tmp = NamedTempFile::new().expect("failed to create temp file");
    let path = tmp.path().to_path_buf();
    let handler = Arc::new(FemtoFileHandler::new(&path).expect("Failed to create file handler"));
    let mut join_handles = vec![];
    for i in 0..10 {
        let h = Arc::clone(&handler);
        join_handles.push(thread::spawn(move || {
            h.expect_handle(FemtoLogRecord::new(
                "core",
                FemtoLevel::Info,
                &format!("msg{i}"),
            ));
        }));
    }
    for h in join_handles {
        h.join().expect("Thread panicked");
    }
    drop(handler);
    let output = fs::read_to_string(&path).expect("failed to read log output");
    for i in 0..10 {
        assert!(output.contains(&format!("core [INFO] msg{i}")));
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
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"));
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "third"));
    })
    .expect("temporary file handler setup failed");

    assert_eq!(
        output,
        "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n",
    );
}

#[test]
fn file_handler_flush_interval_zero() {
    let cfg = HandlerConfig {
        capacity: 8,
        flush_interval: 0,
        overflow_policy: OverflowPolicy::Drop,
    };
    let tmp = NamedTempFile::new().expect("failed to create temp file");
    let result = FemtoFileHandler::with_capacity_flush_policy(tmp.path(), DefaultFormatter, cfg);
    let Err(err) = result else {
        panic!("expected invalid flush_interval");
    };
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(err.to_string(), "flush_interval must be greater than zero");
}

#[test]
fn file_handler_flush_interval_one() {
    let output = with_temp_file_handler_flush(8, 1, |h| {
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "message"));
    })
    .expect("temporary file handler setup failed");
    assert_eq!(output, "core [INFO] message\n");
}

#[test]
fn blocking_policy_waits_for_space() {
    let harness = OverflowHarness::new(1, OverflowPolicy::Block);
    let handler = Arc::new(harness.handler);

    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
    let h = Arc::clone(&handler);
    let t = thread::spawn(move || {
        h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"));
    });
    thread::sleep(Duration::from_millis(50));
    assert!(!t.is_finished());
    harness.start.wait();
    t.join().expect("blocked producer thread panicked");
    assert!(
        handler.flush(),
        "flush should succeed after the blocked producer completes",
    );

    let output = read_output(&harness.buffer).expect("buffer output should be valid UTF-8");
    assert!(
        output.contains("core [INFO] first"),
        "output should contain the first record",
    );
    assert!(
        output.contains("core [INFO] second"),
        "output should contain the second record",
    );
    let first_idx = output
        .find("core [INFO] first")
        .expect("first record missing from output");
    let second_idx = output
        .find("core [INFO] second")
        .expect("second record missing from output");
    assert!(
        first_idx < second_idx,
        "the first record should precede the second record",
    );
}

#[test]
fn timeout_policy_gives_up() {
    let harness = OverflowHarness::new(1, OverflowPolicy::Timeout(Duration::from_millis(50)));

    harness
        .handler
        .expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
    let start_time = Instant::now();
    let err = harness
        .handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"))
        .expect_err("second record should time out");
    assert_eq!(err, HandlerError::Timeout(Duration::from_millis(50)));
    assert!(
        start_time.elapsed() >= Duration::from_millis(50),
        "timeout should wait for at least the configured duration",
    );
    harness.start.wait();
    // Flushing waits for the worker to acknowledge, so the queued record is
    // guaranteed to have reached the buffer by the time this returns.
    assert!(
        harness.handler.flush(),
        "flush should succeed after the worker resumes",
    );
    assert!(
        harness
            .output()
            .expect("buffer output should be valid UTF-8")
            .contains("core [INFO] first"),
        "the queued record should still be written once the worker resumes",
    );
}
