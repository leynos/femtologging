//! Unit tests for the file handler implementation.
//!
//! These tests verify the wiring between configuration and worker threads as
//! well as basic flushing behaviour.

use super::*;
use serial_test::serial;
use std::io::{self, Write};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self {
        Self(buf)
    }

    fn contents(buf: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buf.lock().expect("lock").clone()).expect("invalid UTF-8")
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("lock").write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().expect("lock").flush()
    }
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
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = SharedBuf::new(Arc::clone(&buffer));
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

    let output = String::from_utf8(
        buffer
            .lock()
            .expect("failed to acquire buffer lock for read")
            .clone(),
    )
    .expect("buffer contained invalid UTF-8");
    assert_eq!(output, "core [INFO] test\n");
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
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = TestConfig::new(SharedBuf::new(Arc::clone(&buffer)), DefaultFormatter);
    let barrier = Arc::new(Barrier::new(2));
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    cfg.overflow_policy = OverflowPolicy::Drop;
    cfg.start_barrier = Some(Arc::clone(&barrier));
    let handler = FemtoFileHandler::with_writer_for_test(cfg);

    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "second"));
    barrier.wait();
    drop(handler);

    assert_eq!(SharedBuf::contents(&buffer), "core [INFO] first\n");
}

#[test]
fn femto_file_handler_queue_overflow_block_policy() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = TestConfig::new(SharedBuf::new(Arc::clone(&buffer)), DefaultFormatter);
    let barrier = Arc::new(Barrier::new(2));
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    cfg.overflow_policy = OverflowPolicy::Block;
    cfg.start_barrier = Some(Arc::clone(&barrier));
    let handler = Arc::new(FemtoFileHandler::with_writer_for_test(cfg));

    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    let h = Arc::clone(&handler);
    let t = thread::spawn(move || {
        h.handle(FemtoLogRecord::new("core", "INFO", "second"));
    });
    thread::sleep(Duration::from_millis(50));
    assert!(!t.is_finished());
    barrier.wait();
    t.join().unwrap();
    drop(handler);

    let out = SharedBuf::contents(&buffer);
    assert!(out.contains("core [INFO] first"));
    assert!(out.contains("core [INFO] second"));
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
