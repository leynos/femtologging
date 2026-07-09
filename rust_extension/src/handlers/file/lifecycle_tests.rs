//! Lifecycle tests for the file handler: worker-thread failure handling
//! and flush/close idempotency.
//!
//! Split from `tests.rs` to keep each test module within the size limit.

use super::*;
use crate::formatter::DefaultFormatter;
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use std::io::{self, ErrorKind, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

#[test]
fn femto_file_handler_worker_thread_failure() {
    #[derive(Clone)]
    struct BlockingWriter {
        buf: Arc<Mutex<Vec<u8>>>,
        barrier: Arc<Barrier>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buf.lock().unwrap().write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.barrier.wait();
            self.buf.lock().unwrap().flush()
        }
    }

    impl Seek for BlockingWriter {
        fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
            Err(io::Error::new(
                ErrorKind::Unsupported,
                "seek unsupported for BlockingWriter",
            ))
        }
    }

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
    struct TestWriter {
        flushed: Arc<AtomicU32>,
        closed: Arc<AtomicU32>,
    }

    impl Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    impl Seek for TestWriter {
        fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
            Err(io::Error::new(
                ErrorKind::Unsupported,
                "seek unsupported for TestWriter",
            ))
        }
    }

    impl Drop for TestWriter {
        fn drop(&mut self) {
            self.closed.fetch_add(1, Ordering::Relaxed);
        }
    }

    let flushed = Arc::new(AtomicU32::new(0));
    let closed = Arc::new(AtomicU32::new(0));
    let writer = TestWriter {
        flushed: Arc::clone(&flushed),
        closed: Arc::clone(&closed),
    };

    // Disable periodic flushing to ensure deterministic counter checks.
    let handler_cfg = HandlerConfig {
        capacity: 10,
        flush_interval: 0,
        overflow_policy: OverflowPolicy::Block,
    };
    let mut handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<TestWriter>::default(),
    );

    assert!(handler.flush());
    assert_eq!(flushed.load(Ordering::Relaxed), 1);

    assert!(handler.flush());
    assert_eq!(flushed.load(Ordering::Relaxed), 2);

    handler.close();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    // Expect two manual flushes plus one triggered during shutdown
    assert_eq!(flushed.load(Ordering::Relaxed), 3);

    handler.close();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert_eq!(flushed.load(Ordering::Relaxed), 3);

    assert!(
        !handler.flush(),
        "flush after close should be a no-op and report failure"
    );
    assert_eq!(flushed.load(Ordering::Relaxed), 3);

    assert!(!handler.flush());
    // Ensure counters remain unchanged after the no-op flush
    assert_eq!(flushed.load(Ordering::Relaxed), 3);
    assert_eq!(closed.load(Ordering::Relaxed), 1);

    drop(handler);
    assert_eq!(flushed.load(Ordering::Relaxed), 3);
    assert_eq!(closed.load(Ordering::Relaxed), 1);
}
