//! Regression tests for file handler flush acknowledgement semantics.
//!
//! These tests ensure each flush call waits for its own worker
//! acknowledgement rather than reusing a stale ack from an earlier timed-out
//! flush.

use super::*;
use std::io::{self, ErrorKind, Seek, SeekFrom, Write};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

#[derive(Clone)]
struct BlockingFlushWriter {
    state: Arc<BlockingFlushState>,
}

struct BlockingFlushState {
    buffer: Mutex<Vec<u8>>,
    flush_calls: Mutex<usize>,
    released_flushes: Mutex<usize>,
    release_cvar: Condvar,
    started_tx: mpsc::Sender<usize>,
}

impl BlockingFlushWriter {
    /// Build a writer whose flush calls block until the test releases them.
    fn new(started_tx: mpsc::Sender<usize>) -> Self {
        Self {
            state: Arc::new(BlockingFlushState {
                buffer: Mutex::new(Vec::new()),
                flush_calls: Mutex::new(0),
                released_flushes: Mutex::new(0),
                release_cvar: Condvar::new(),
                started_tx,
            }),
        }
    }

    /// Release all blocked flushes up to and including `flush_number`.
    fn release_flushes_through(&self, flush_number: usize) {
        let mut released = self
            .state
            .released_flushes
            .lock()
            .expect("released flush counter must not be poisoned");
        *released = flush_number;
        self.state.release_cvar.notify_all();
    }

    /// Release every blocked flush call.
    fn release_all_flushes(&self) {
        self.release_flushes_through(usize::MAX);
    }
}

impl Write for BlockingFlushWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut buffer = self
            .state
            .buffer
            .lock()
            .expect("buffer mutex must not be poisoned");
        buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let flush_number = {
            let mut flush_calls = self
                .state
                .flush_calls
                .lock()
                .expect("flush counter must not be poisoned");
            *flush_calls += 1;
            *flush_calls
        };
        self.state
            .started_tx
            .send(flush_number)
            .expect("flush start notification receiver must stay connected");

        let mut released = self
            .state
            .released_flushes
            .lock()
            .expect("released flush counter must not be poisoned");
        while *released < flush_number {
            released = self
                .state
                .release_cvar
                .wait(released)
                .expect("released flush counter must not be poisoned");
        }
        Ok(())
    }
}

impl Seek for BlockingFlushWriter {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "seek unsupported for BlockingFlushWriter",
        ))
    }
}

/// Ensure each flush waits on its own acknowledgement channel.
#[test]
fn flush_waits_for_its_own_acknowledgement() {
    let (started_tx, started_rx) = mpsc::channel();
    let writer = BlockingFlushWriter::new(started_tx);
    let mut config = TestConfig::new(writer.clone(), DefaultFormatter);
    config.capacity = 4;
    config.flush_interval = 16;
    config.overflow_policy = OverflowPolicy::Block;

    let handler = Arc::new(FemtoFileHandler::with_writer_for_test(config));

    let first_handler = Arc::clone(&handler);
    let (first_done_tx, first_done_rx) = mpsc::channel();
    let first_flush = thread::spawn(move || {
        first_done_tx
            .send(first_handler.flush())
            .expect("first flush result receiver must stay connected");
    });

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first flush must reach the writer"),
        1
    );
    assert!(
        !first_done_rx
            .recv_timeout(Duration::from_millis(1_200))
            .expect("first flush must time out while the writer is blocked"),
        "first flush should fail once its private ack receiver times out",
    );
    first_flush
        .join()
        .expect("first flush worker thread must complete cleanly");

    writer.release_flushes_through(1);

    let second_handler = Arc::clone(&handler);
    let (second_done_tx, second_done_rx) = mpsc::channel();
    let second_flush = thread::spawn(move || {
        second_done_tx
            .send(second_handler.flush())
            .expect("second flush result receiver must stay connected");
    });

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second flush must reach the writer"),
        2
    );
    assert!(
        second_done_rx
            // Slow CI workers can take longer to schedule the waiting flush,
            // so keep a wider margin before declaring the stale-ack guard
            // broken.
            .recv_timeout(Duration::from_millis(500))
            .is_err(),
        "second flush must wait for its own acknowledgement instead of consuming a stale ack",
    );

    writer.release_flushes_through(2);
    assert!(
        second_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second flush must complete after its writer flush is released"),
        "second flush should succeed once its own ack arrives",
    );
    second_flush
        .join()
        .expect("second flush worker thread must complete cleanly");

    writer.release_all_flushes();
    let mut handler = Arc::try_unwrap(handler).unwrap_or_else(|_| {
        panic!("flush test must release all handler references before shutdown")
    });
    handler.close();
}
