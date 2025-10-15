#![cfg(all(test, feature = "python"))]
//! Shared helper types and fixtures for file handler tests.
//!
//! The utilities defined here allow tests to coordinate worker threads and
//! capture writes using simple in-memory buffers.

use std::io::{self, ErrorKind, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::thread;

use super::super::{
    DefaultFormatter, FemtoFileHandler, FemtoLogRecord, OverflowPolicy, RotationStrategy,
    TestConfig,
};

#[derive(Clone, Default)]
pub(super) struct SharedBuf {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuf {
    /// Return the UTF-8 contents of the buffer.
    ///
    /// # Panics
    ///
    /// Panics if the buffer contains invalid UTF-8 or the lock is poisoned.
    pub(super) fn contents(&self) -> String {
        String::from_utf8(
            self.buffer
                .lock()
                .expect("SharedBuf lock poisoned while reading")
                .clone(),
        )
        .expect("SharedBuf contained invalid UTF-8")
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer
            .lock()
            .expect("SharedBuf lock poisoned during write")
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buffer
            .lock()
            .expect("SharedBuf lock poisoned during flush")
            .flush()
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

/// Rotation strategy that counts the number of `before_write` invocations.
///
/// Used in tests to verify that rotation hooks are called the expected number
/// of times without rotating the writer.
pub(super) struct CountingRotation {
    calls: Arc<AtomicUsize>,
}

impl CountingRotation {
    /// Construct a new counter-backed rotation strategy.
    pub(super) fn new(calls: Arc<AtomicUsize>) -> Self {
        Self { calls }
    }
}

impl RotationStrategy<SharedBuf> for CountingRotation {
    fn before_write(&mut self, _writer: &mut SharedBuf, _formatted: &str) -> io::Result<bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}

/// Rotation strategy that flips a shared flag when invoked.
///
/// Tests use this to confirm rotation hooks run at specific lifecycle moments.
pub(super) struct FlagRotation {
    flag: Arc<AtomicBool>,
}

impl FlagRotation {
    /// Construct a new flag-backed rotation strategy.
    pub(super) fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

impl RotationStrategy<std::io::Cursor<Vec<u8>>> for FlagRotation {
    fn before_write(
        &mut self,
        _writer: &mut std::io::Cursor<Vec<u8>>,
        _formatted: &str,
    ) -> io::Result<bool> {
        self.flag.store(true, Ordering::SeqCst);
        Ok(false)
    }
}

/// Configure a handler for overflow testing with a minimal queue capacity.
///
/// Returns a tuple of:
/// - `SharedBuf`: in-memory buffer receiving formatted records
/// - `Arc<Barrier>`: synchronisation barrier for coordinating worker start
/// - `FemtoFileHandler`: configured handler under test
///
/// The handler is configured with capacity 1, immediate flush, and the supplied
/// overflow policy to trigger overflow scenarios deterministically.
pub(super) fn setup_overflow_test(
    policy: OverflowPolicy,
) -> (SharedBuf, Arc<Barrier>, FemtoFileHandler) {
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

/// Spawn a thread that sends a single record to the handler after barrier
/// synchronisation.
///
/// Returns a tuple of:
/// - `Arc`: synchronisation barrier to release the spawned thread
/// - `mpsc::Receiver<()>`: channel signalling record send completion
/// - `JoinHandle<()>`: handle to the spawned thread
///
/// The spawned thread waits on the barrier, sends the record via
/// `handler.handle()`, and signals completion using the receiver. Use the
/// receiver to confirm the send occurred and the join handle to await thread
/// completion.
pub(super) fn spawn_record_thread(
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
