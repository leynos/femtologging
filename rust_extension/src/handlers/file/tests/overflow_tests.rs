#![cfg(all(test, feature = "python"))]
//! Tests covering queue overflow behaviour for the file handler worker.

use super::super::*;
use super::test_support::{setup_overflow_test, spawn_record_thread};
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use serial_test::serial;
use std::sync::{Arc, Barrier};
use std::time::Duration;

/// Assert that a drop scenario records the expected warning message.
///
/// The helper installs the test logger, queues an initial record, runs the
/// caller-provided setup hook, and then sends a second record that must fail.
/// It verifies that the error matches the expectation and that a single warning
/// was emitted with the required text.
fn assert_drop_warning_logged<Setup>(
    overflow_policy: OverflowPolicy,
    expected_error: HandlerError,
    expected_message: &str,
    setup: Setup,
) where
    Setup: FnOnce(&mut FemtoFileHandler, &Arc<Barrier>) -> bool,
{
    install_test_logger();
    let (_buffer, start_barrier, mut handler) = setup_overflow_test(overflow_policy);

    handler
        .handle(FemtoLogRecord::new("core", "INFO", "first"))
        .expect("first record queued");

    let barrier_released = setup(&mut handler, &start_barrier);

    let err = handler
        .handle(FemtoLogRecord::new("core", "INFO", "second"))
        .expect_err("second record should be dropped");
    assert_eq!(err, expected_error);

    let logs = take_logged_messages();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message, expected_message);

    if !barrier_released {
        start_barrier.wait();
    }

    drop(handler);
}

#[test]
#[serial]
fn femto_file_handler_queue_overflow_drop_policy() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Drop);

    handler
        .handle(FemtoLogRecord::new("core", "INFO", "first"))
        .expect("first record queued");
    let err = handler
        .handle(FemtoLogRecord::new("core", "INFO", "second"))
        .expect_err("second record should overflow");
    assert_eq!(err, HandlerError::QueueFull);
    start_barrier.wait();
    drop(handler);

    assert_eq!(buffer.contents(), "core [INFO] first\n");
}

#[test]
fn femto_file_handler_queue_overflow_block_policy() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Block);
    handler
        .handle(FemtoLogRecord::new("core", "INFO", "first"))
        .expect("first record queued");

    let handler = Arc::new(handler);
    let (send_barrier, done_rx, t) = spawn_record_thread(
        Arc::clone(&handler),
        FemtoLogRecord::new("core", "INFO", "second"),
    );

    send_barrier.wait();
    assert!(done_rx.try_recv().is_err());
    start_barrier.wait();
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker did not finish");
    t.join().expect("join thread");
    drop(handler);

    let out = buffer.contents();
    assert!(out.contains("core [INFO] first"));
    assert!(out.contains("core [INFO] second"));
    let first_idx = out.find("core [INFO] first").expect("first log not found");
    let second_idx = out
        .find("core [INFO] second")
        .expect("second log not found");
    assert!(
        first_idx < second_idx,
        "\"core [INFO] first\" does not appear before \"core [INFO] second\" in output",
    );
}

#[test]
#[serial]
fn femto_file_handler_queue_overflow_drop_policy_large_batch() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Drop);

    let mut successes = 0;
    let mut failures = 0;
    for i in 0..32 {
        let record = FemtoLogRecord::new("core", "INFO", &format!("batch{i}"));
        match handler.handle(record) {
            Ok(()) => successes += 1,
            Err(err) => {
                assert_eq!(err, HandlerError::QueueFull);
                failures += 1;
            }
        }
    }

    assert_eq!(successes, 1, "only the first record should be accepted");
    assert_eq!(
        failures, 31,
        "all subsequent records must overflow the queue"
    );

    start_barrier.wait();
    drop(handler);

    assert_eq!(buffer.contents(), "core [INFO] batch0\n");
}

#[test]
#[serial]
fn femto_file_handler_queue_overflow_block_policy_large_batch() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Block);

    handler
        .handle(FemtoLogRecord::new("core", "INFO", "batch0"))
        .expect("initial record queued");

    let handler = Arc::new(handler);
    let mut send_barriers = Vec::new();
    let mut done_rxs = Vec::new();
    let mut joins = Vec::new();

    for i in 1..32 {
        let (send_barrier, done_rx, handle) = spawn_record_thread(
            Arc::clone(&handler),
            FemtoLogRecord::new("core", "INFO", &format!("batch{i}")),
        );
        send_barriers.push(send_barrier);
        done_rxs.push(done_rx);
        joins.push(handle);
    }

    for barrier in &send_barriers {
        barrier.wait();
    }

    for rx in &done_rxs {
        assert!(rx.try_recv().is_err(), "sender should still be blocked");
    }

    start_barrier.wait();

    for rx in done_rxs {
        rx.recv_timeout(Duration::from_secs(2))
            .expect("worker did not drain queue in time");
    }

    for join in joins {
        join.join().expect("join sender thread");
    }

    drop(handler);

    let lines: Vec<String> = buffer.contents().lines().map(str::to_owned).collect();
    let expected: Vec<String> = (0..32).map(|i| format!("core [INFO] batch{i}")).collect();
    assert_eq!(lines, expected);
}

#[test]
#[serial]
fn femto_file_handler_logs_queue_full_warning() {
    assert_drop_warning_logged(
        OverflowPolicy::Drop,
        HandlerError::QueueFull,
        "FemtoFileHandler: 1 log records dropped because the queue was full",
        |_, _| false,
    );
}

#[test]
#[serial]
fn femto_file_handler_logs_closed_warning() {
    assert_drop_warning_logged(
        OverflowPolicy::Drop,
        HandlerError::Closed,
        "FemtoFileHandler: 1 log records dropped after the handler was closed",
        |handler, barrier| {
            barrier.wait();
            handler.close();
            true
        },
    );
}

#[test]
#[serial]
fn femto_file_handler_logs_timeout_warning() {
    assert_drop_warning_logged(
        OverflowPolicy::Timeout(Duration::from_millis(10)),
        HandlerError::Timeout(Duration::from_millis(10)),
        "FemtoFileHandler: 1 log records dropped after timing out waiting for the worker thread (timeout: Some(10ms))",
        |_, _| false,
    );
}
