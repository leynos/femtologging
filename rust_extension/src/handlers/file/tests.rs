//! Unit tests for the file handler implementation.
//!
//! These tests verify the wiring between configuration and worker threads as
//! well as basic flushing behaviour.

use super::*;
use serial_test::serial;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
struct SharedBuf {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuf {
    /// Return the UTF-8 contents of the buffer.
    fn contents(&self) -> String {
        String::from_utf8(self.buffer.lock().expect("lock").clone()).expect("invalid UTF-8")
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.lock().expect("lock").write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buffer.lock().expect("lock").flush()
    }
}

fn setup_overflow_test(policy: OverflowPolicy) -> (SharedBuf, Arc<Barrier>, FemtoFileHandler) {
    let buffer = SharedBuf::default();
    let start_barrier = Arc::new(Barrier::new(2));
    let mut cfg = TestConfig::new(buffer.clone(), DefaultFormatter);
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    cfg.overflow_policy = policy;
    cfg.start_barrier = Some(Arc::clone(&start_barrier));
    let handler = FemtoFileHandler::with_writer_for_test(cfg);
    (buffer, start_barrier, handler)
}

fn spawn_record_thread(
    handler: Arc<FemtoFileHandler>,
    record: FemtoLogRecord,
) -> (Arc<Barrier>, mpsc::Receiver<()>, thread::JoinHandle<()>) {
    let (done_tx, done_rx) = mpsc::channel();
    let send_barrier = Arc::new(Barrier::new(2));
    let h = Arc::clone(&handler);
    let sb = Arc::clone(&send_barrier);
    let handle = thread::spawn(move || {
        sb.wait();
        h.handle(record);
        done_tx.send(()).expect("send done");
    });
    (send_barrier, done_rx, handle)
}

#[test]
fn worker_config_from_handlerconfig_copies_values() {
    let cfg = HandlerConfig {
        capacity: 42,
        flush_interval: 7,
        overflow_policy: OverflowPolicy::Drop,
    };
    let worker = WorkerConfig::from(&cfg);
    assert_eq!(worker.capacity, 42);
    assert_eq!(worker.flush_interval, 7);
    assert!(worker.start_barrier.is_none());
}

#[test]
fn build_from_worker_wires_handler_components() {
    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let worker_cfg = WorkerConfig {
        capacity: 1,
        flush_interval: 1,
        start_barrier: None,
    };
    let policy = OverflowPolicy::Block;
    let mut handler =
        FemtoFileHandler::build_from_worker(writer, DefaultFormatter, worker_cfg, policy);

    assert!(handler.tx.is_some());
    assert!(handler.handle.is_some());
    assert_eq!(handler.overflow_policy, policy);

    let tx = handler.tx.take().expect("tx missing");
    let done_rx = handler.done_rx.clone();
    let handle = handler.handle.take().expect("handle missing");

    tx.send(FileCommand::Record(FemtoLogRecord::new(
        "core", "INFO", "test",
    )))
    .expect("send");
    drop(tx);

    assert!(done_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok());
    handle.join().expect("worker thread");

    assert_eq!(buffer.contents(), "core [INFO] test\n");
}

#[test]
fn femto_file_handler_invalid_file_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("missing").join("out.log");
    assert!(FemtoFileHandler::new(&path).is_err());
}

#[test]
#[serial]
fn femto_file_handler_queue_overflow_drop_policy() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Drop);

    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "second"));
    start_barrier.wait();
    drop(handler);

    assert_eq!(buffer.contents(), "core [INFO] first\n");
}

#[test]
fn femto_file_handler_queue_overflow_block_policy() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Block);
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));

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
    handler.handle(FemtoLogRecord::new("core", "INFO", "slow"));
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
            self.flushed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    impl Drop for TestWriter {
        fn drop(&mut self) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    let flushed = Arc::new(AtomicU32::new(0));
    let closed = Arc::new(AtomicU32::new(0));
    let writer = TestWriter {
        flushed: Arc::clone(&flushed),
        closed: Arc::clone(&closed),
    };

    let worker_cfg = WorkerConfig {
        capacity: 10,
        flush_interval: 1,
        start_barrier: None,
    };
    let mut handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        worker_cfg,
        OverflowPolicy::Block,
    );

    assert!(handler.flush());
    assert_eq!(flushed.load(Ordering::SeqCst), 1);

    assert!(handler.flush());
    assert_eq!(flushed.load(Ordering::SeqCst), 2);

    handler.close();
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(flushed.load(Ordering::SeqCst), 3);

    handler.close();
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(flushed.load(Ordering::SeqCst), 3);

    assert!(!handler.flush());
    // Ensure counters remain unchanged after the no-op flush
    assert_eq!(flushed.load(Ordering::SeqCst), 3);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}
