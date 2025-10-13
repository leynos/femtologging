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
    pub(super) fn contents(&self) -> String {
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

pub(super) struct CountingRotation {
    calls: Arc<AtomicUsize>,
}

impl CountingRotation {
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

pub(super) struct FlagRotation {
    flag: Arc<AtomicBool>,
}

impl FlagRotation {
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
