//! Lifecycle tests for the file handler: worker-thread failure handling
//! and flush/close idempotency.
//!
//! Split from `tests.rs` to keep each test module within the size limit.

use super::test_support::impl_unsupported_seek;
use super::*;
use crate::formatter::DefaultFormatter;
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

struct LifecycleWriter {
    flushed: Arc<AtomicU32>,
    closed: Arc<AtomicU32>,
}

impl Write for LifecycleWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl_unsupported_seek!(LifecycleWriter);

impl Drop for LifecycleWriter {
    fn drop(&mut self) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }
}

fn make_lifecycle_handler(flushed: Arc<AtomicU32>, closed: Arc<AtomicU32>) -> FemtoFileHandler {
    let writer = LifecycleWriter { flushed, closed };
    let handler_cfg = HandlerConfig {
        capacity: 10,
        flush_interval: 0,
        overflow_policy: OverflowPolicy::Block,
    };
    FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<LifecycleWriter>::default(),
    )
}

fn assert_flush_count(flushed: &AtomicU32, expected: u32) {
    assert_eq!(flushed.load(Ordering::Relaxed), expected);
}

fn verify_manual_flushes(handler: &mut FemtoFileHandler, flushed: &AtomicU32) {
    assert!(handler.flush());
    assert_flush_count(flushed, 1);

    assert!(handler.flush());
    assert_flush_count(flushed, 2);
}

fn verify_close_is_idempotent(
    handler: &mut FemtoFileHandler,
    flushed: &AtomicU32,
    closed: &AtomicU32,
) {
    handler.close();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    // Expect two manual flushes plus one triggered during shutdown.
    assert_flush_count(flushed, 3);

    handler.close();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert_flush_count(flushed, 3);
}

fn verify_closed_handler_is_noop(
    handler: &mut FemtoFileHandler,
    flushed: &AtomicU32,
    closed: &AtomicU32,
) {
    assert!(
        !handler.flush(),
        "flush after close should be a no-op and report failure"
    );
    assert_flush_count(flushed, 3);

    assert!(!handler.flush());
    // Ensure counters remain unchanged after the no-op flush.
    assert_flush_count(flushed, 3);
    assert_eq!(closed.load(Ordering::Relaxed), 1);
}

#[test]
fn femto_file_handler_worker_thread_failure() {
    #[derive(Clone)]
    struct BlockingWriter {
        buf: Arc<Mutex<Vec<u8>>>,
        barrier: Arc<Barrier>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            // Recover from poisoning: the buffer contents remain valid data.
            self.buf
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.barrier.wait();
            // Recover from poisoning: the buffer contents remain valid data.
            self.buf
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .flush()
        }
    }

    impl_unsupported_seek!(BlockingWriter);

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let barrier = Arc::new(Barrier::new(2));
    let mut cfg = TestConfig::new(
        BlockingWriter {
            buf: Arc::clone(&buffer),
            barrier: Arc::clone(&barrier),
        },
        DefaultFormatter,
    );
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    let handler = FemtoFileHandler::with_writer_for_test(cfg);
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "slow"))
        .expect("record queued");
    let start = Instant::now();
    drop(handler);
    assert!(start.elapsed() < Duration::from_millis(1500));
    barrier.wait();
}

#[test]
fn femto_file_handler_flush_and_close_idempotency() {
    let flushed = Arc::new(AtomicU32::new(0));
    let closed = Arc::new(AtomicU32::new(0));
    let mut handler = make_lifecycle_handler(Arc::clone(&flushed), Arc::clone(&closed));

    verify_manual_flushes(&mut handler, &flushed);
    verify_close_is_idempotent(&mut handler, &flushed, &closed);
    verify_closed_handler_is_noop(&mut handler, &flushed, &closed);

    drop(handler);
    assert_flush_count(&flushed, 3);
    assert_eq!(closed.load(Ordering::Relaxed), 1);
}
