#![cfg(all(test, feature = "python"))]
//! Tests covering queue overflow behaviour for the file handler worker.

use super::super::*;
use super::test_support::{setup_overflow_test, spawn_record_thread};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;

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
