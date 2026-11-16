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

#[test]
fn femto_file_handler_worker_thread_failure() {
    /// Writer blocked at flush so dropping the handler exercises worker failure.
    #[derive(Clone)]
    struct BlockingWriter {
        /// Captures bytes written by the file worker.
        buf: Arc<Mutex<Vec<u8>>>,
        /// Synchronizes the test with the worker's flush operation.
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

/// Writer double that counts explicit and shutdown-triggered flushes and
/// records its own destruction.
struct CountingWriter {
    /// Shared count of calls to `flush`.
    flushed: Arc<AtomicU32>,
    /// Shared count of writer drops, used to check close idempotency.
    closed: Arc<AtomicU32>,
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl_unsupported_seek!(CountingWriter);

impl Drop for CountingWriter {
    fn drop(&mut self) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Assert the writer saw `expected_flushes` flushes and `expected_closes` drops.
fn assert_counts(
    flushed: &AtomicU32,
    closed: &AtomicU32,
    expected_flushes: u32,
    expected_closes: u32,
) {
    assert_eq!(
        flushed.load(Ordering::Relaxed),
        expected_flushes,
        "unexpected flush count"
    );
    assert_eq!(
        closed.load(Ordering::Relaxed),
        expected_closes,
        "unexpected writer drop count"
    );
}

#[test]
fn femto_file_handler_flush_and_close_idempotency() {
    let flushed = Arc::new(AtomicU32::new(0));
    let closed = Arc::new(AtomicU32::new(0));
    let writer = CountingWriter {
        flushed: Arc::clone(&flushed),
        closed: Arc::clone(&closed),
    };

    // Disable periodic flushing to ensure deterministic counter checks.
    let handler_cfg = HandlerConfig {
        capacity: 10,
        flush_interval: 0,
        overflow_policy: OverflowPolicy::Block,
    };
    let handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<CountingWriter>::default(),
    );

    assert!(handler.flush(), "first explicit flush should succeed");
    assert_counts(&flushed, &closed, 1, 0);

    assert!(handler.flush(), "second explicit flush should succeed");
    assert_counts(&flushed, &closed, 2, 0);

    handler.close();
    // Two manual flushes plus one triggered during shutdown.
    assert_counts(&flushed, &closed, 3, 1);

    handler.close();
    assert_counts(&flushed, &closed, 3, 1);

    assert!(
        !handler.flush(),
        "flush after close should be a no-op and report failure"
    );
    assert_counts(&flushed, &closed, 3, 1);

    assert!(
        !handler.flush(),
        "a second no-op flush must also report failure"
    );
    // Ensure counters remain unchanged after the no-op flushes.
    assert_counts(&flushed, &closed, 3, 1);

    drop(handler);
    assert_counts(&flushed, &closed, 3, 1);
}
