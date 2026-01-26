//! Unit tests for the file handler implementation.
//!
//! These tests verify the wiring between configuration and worker threads as
//! well as basic flushing behaviour.

use super::test_support::{install_test_logger, take_logged_messages};
use super::*;
use crate::handler::HandlerError;
use log::Level;
use serial_test::serial;
use std::io::{self, Cursor, ErrorKind, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
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

impl Seek for SharedBuf {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "seek unsupported for SharedBuf",
        ))
    }
}

struct CountingRotation {
    calls: Arc<AtomicUsize>,
}

impl CountingRotation {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self { calls }
    }
}

impl RotationStrategy<SharedBuf> for CountingRotation {
    fn before_write(&mut self, _writer: &mut SharedBuf, _formatted: &str) -> io::Result<bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}

struct FlagRotation {
    flag: Arc<AtomicBool>,
}

impl FlagRotation {
    fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

impl RotationStrategy<Cursor<Vec<u8>>> for FlagRotation {
    fn before_write(
        &mut self,
        _writer: &mut Cursor<Vec<u8>>,
        _formatted: &str,
    ) -> io::Result<bool> {
        self.flag.store(true, Ordering::SeqCst);
        Ok(false)
    }
}

#[test]
fn builder_options_default_provides_noop_rotation() {
    let mut options: BuilderOptions<Cursor<Vec<u8>>> = BuilderOptions::default();
    assert!(options.start_barrier.is_none());
    let mut writer = Cursor::new(Vec::new());
    assert!(options.rotation.before_write(&mut writer, "entry").is_ok());
}

#[test]
fn builder_options_new_stores_rotation_and_barrier() {
    let flag = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(1));
    let mut options = BuilderOptions::<Cursor<Vec<u8>>, FlagRotation>::new(
        FlagRotation::new(Arc::clone(&flag)),
        Some(Arc::clone(&barrier)),
    );

    let stored = options.start_barrier.take().expect("missing barrier");
    assert!(Arc::ptr_eq(&stored, &barrier));

    let mut writer = Cursor::new(Vec::new());
    options
        .rotation
        .before_write(&mut writer, "check")
        .expect("rotation should succeed");
    assert!(flag.load(Ordering::SeqCst));
}

#[test]
fn build_from_worker_invokes_rotation_strategy() {
    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let handler_cfg = HandlerConfig {
        capacity: 4,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let rotation = CountingRotation::new(Arc::clone(&calls));
    let mut handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<SharedBuf, _>::new(rotation, None),
    );

    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "one"))
        .expect("record one queued");
    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "two"))
        .expect("record two queued");

    assert!(handler.flush());
    handler.close();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(buffer.contents(), "core [INFO] one\ncore [INFO] two\n");
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
        h.handle(record).expect("record send");
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
    let handler_cfg = HandlerConfig {
        capacity: 1,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let policy = handler_cfg.overflow_policy;
    let mut handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<SharedBuf>::default(),
    );

    assert!(handler.tx.is_some());
    assert!(handler.handle.is_some());
    assert_eq!(handler.overflow_policy, policy);

    let tx = handler.tx.take().expect("tx missing");
    let done_rx = handler.done_rx.clone();
    let handle = handler.handle.take().expect("handle missing");

    tx.send(FileCommand::Record(Box::new(FemtoLogRecord::new(
        "core",
        FemtoLevel::Info,
        "test",
    ))))
    .expect("send");
    drop(tx);

    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok()
    );
    handle.join().expect("worker thread");

    assert_eq!(buffer.contents(), "core [INFO] test\n");
}

#[test]
#[serial]
fn worker_writes_record_when_rotation_fails() {
    struct FailingRotation;

    impl RotationStrategy<SharedBuf> for FailingRotation {
        fn before_write(&mut self, _writer: &mut SharedBuf, _formatted: &str) -> io::Result<bool> {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "failing rotation for test",
            ))
        }
    }

    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let handler_cfg = HandlerConfig {
        capacity: 1,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let options = BuilderOptions::<SharedBuf, FailingRotation>::new(FailingRotation, None);
    install_test_logger();
    let mut handler =
        FemtoFileHandler::build_from_worker(writer, DefaultFormatter, handler_cfg, options);

    handler
        .handle(FemtoLogRecord::new(
            "core",
            FemtoLevel::Info,
            "after rotation failure",
        ))
        .expect("record queued after rotation warning");
    assert!(
        handler.flush(),
        "flush should succeed even if rotation reported an error",
    );
    handler.close();

    let logs = take_logged_messages();
    assert!(
        logs.iter().any(|record| {
            record.level == Level::Error
                && record
                    .message
                    .contains("FemtoFileHandler rotation error; writing record without rotating")
        }),
        "rotation error should be logged"
    );

    assert_eq!(buffer.contents(), "core [INFO] after rotation failure\n");
}

#[test]
fn femto_file_handler_invalid_file_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("missing").join("out.log");
    assert!(FemtoFileHandler::new(&path).is_err());
}

#[test]
fn femto_file_handler_rejects_zero_capacity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("out.log");
    let cfg = HandlerConfig {
        capacity: 0,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Drop,
    };

    let result = FemtoFileHandler::with_capacity_flush_policy(&path, DefaultFormatter, cfg);
    assert!(result.is_err(), "zero capacity should be rejected");
    let err = result.err().expect("missing error for zero capacity");

    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(err.to_string(), "capacity must be greater than zero");
}

#[test]
#[serial]
fn femto_file_handler_queue_overflow_drop_policy() {
    let (buffer, start_barrier, handler) = setup_overflow_test(OverflowPolicy::Drop);

    handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"))
        .expect("first record queued");
    let err = handler
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"))
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
        .handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"))
        .expect("first record queued");

    let handler = Arc::new(handler);
    let (send_barrier, done_rx, t) = spawn_record_thread(
        Arc::clone(&handler),
        FemtoLogRecord::new("core", FemtoLevel::Info, "second"),
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
