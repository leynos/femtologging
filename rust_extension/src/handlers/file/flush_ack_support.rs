//! Harness for the file handler's flush-acknowledgement tests.
//!
//! Those tests need a worker that blocks inside the writer's `flush`, so an
//! acknowledgement can be withheld and released on demand. Building that
//! handler, releasing the blocked writer, and shutting the handler down are
//! the same steps every time; keeping them here means a failing assertion
//! still releases the worker and closes the handler instead of leaving both
//! behind for the rest of the run.

use std::io::{self, ErrorKind, Write};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rstest::fixture;

use super::test_support::impl_unsupported_seek;
use super::*;

/// Cloneable writer whose flush calls wait for explicit test release.
#[derive(Clone)]
pub(super) struct BlockingFlushWriter {
    /// Shared state keeps write and flush observations consistent across clones.
    state: Arc<BlockingFlushState>,
}

/// Coordination shared between the writer and the test controlling flushes.
struct BlockingFlushState {
    /// Bytes accepted by `write`, protected because worker and test inspect it.
    buffer: Mutex<Vec<u8>>,
    /// Number assigned to each flush invocation.
    flush_calls: Mutex<usize>,
    /// Highest flush number the test has released.
    released_flushes: Mutex<usize>,
    /// Wakes blocked flush calls when the test advances `released_flushes`.
    release_cvar: Condvar,
    /// Reports each started flush to the coordinating test.
    started_tx: mpsc::Sender<usize>,
}

impl BlockingFlushWriter {
    /// Build a writer whose flush calls block until the test releases them.
    pub(super) fn new(started_tx: mpsc::Sender<usize>) -> Self {
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
    pub(super) fn release_flushes_through(&self, flush_number: usize) {
        // Recover from poisoning: the counter is always valid data.
        let mut released = self
            .state
            .released_flushes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *released = flush_number;
        self.state.release_cvar.notify_all();
    }

    /// Release every blocked flush call.
    pub(super) fn release_all_flushes(&self) {
        self.release_flushes_through(usize::MAX);
    }
}

impl Write for BlockingFlushWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Recover from poisoning: the buffer contents remain valid data.
        let mut buffer = self
            .state
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let flush_number = {
            // Recover from poisoning: the counter is always valid data.
            let mut flush_calls = self
                .state
                .flush_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *flush_calls += 1;
            *flush_calls
        };
        // Surface a disconnected test receiver as an I/O error so the
        // calling test fails with context instead of panicking here.
        self.state.started_tx.send(flush_number).map_err(|_| {
            io::Error::new(
                ErrorKind::BrokenPipe,
                "flush start notification receiver disconnected",
            )
        })?;

        let mut released = self
            .state
            .released_flushes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *released < flush_number {
            released = self
                .state
                .release_cvar
                .wait(released)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        Ok(())
    }
}

impl_unsupported_seek!(BlockingFlushWriter);

/// A file handler whose worker is stalled inside the writer's flush.
pub(super) struct StalledFlushHandler {
    /// The handler under test, closed when this harness drops.
    handler: Option<Arc<FemtoFileHandler>>,
    /// Writer whose flush calls stay blocked until the test releases them.
    pub(super) writer: BlockingFlushWriter,
    /// Reports each flush the worker starts, numbered from one.
    pub(super) started: mpsc::Receiver<usize>,
    /// Completes when the stalling flush reports its outcome.
    stall_done: Option<mpsc::Receiver<bool>>,
    /// The thread running the stalling flush, joined on drop.
    stall: Option<thread::JoinHandle<()>>,
}

impl StalledFlushHandler {
    /// Build a handler with `capacity` queue slots and a stalling worker.
    ///
    /// The stalling flush occupies the worker, so anything queued behind it
    /// cannot be acknowledged and the test can decide when the writer moves.
    fn new(capacity: usize) -> Self {
        let (started_tx, started) = mpsc::channel();
        let writer = BlockingFlushWriter::new(started_tx);
        let mut config = TestConfig::new(writer.clone(), DefaultFormatter);
        config.capacity = capacity;
        config.flush_interval = 16;
        config.overflow_policy = OverflowPolicy::Block;

        let handler = Arc::new(FemtoFileHandler::with_writer_for_test(config));
        let stalling = Arc::clone(&handler);
        let (stall_done_tx, stall_done) = mpsc::channel();
        let stall = thread::spawn(move || {
            stall_done_tx
                .send(stalling.flush())
                .expect("stall result receiver must stay connected");
        });
        Self {
            handler: Some(handler),
            writer,
            started,
            stall_done: Some(stall_done),
            stall: Some(stall),
        }
    }

    /// The handler under test.
    ///
    /// Handed out as a borrow so this harness stays the only owner and can
    /// close the handler on drop.
    pub(super) fn handler(&self) -> &FemtoFileHandler {
        self.handler
            .as_deref()
            .expect("the harness owns its handler until it drops")
    }

    /// Wait for the stalling flush to report, then join its thread.
    ///
    /// Returns `None` when the flush neither reports nor disconnects within
    /// two seconds, which the calling test treats as a failure.
    pub(super) fn stall_outcome(&mut self) -> Option<bool> {
        let done = self.stall_done.take()?;
        let outcome = done.recv_timeout(Duration::from_secs(2)).ok();
        if let Some(stall) = self.stall.take() {
            let _ = stall.join();
        }
        outcome
    }
}

impl Drop for StalledFlushHandler {
    fn drop(&mut self) {
        // Release the writer first: the stalling flush can only report once
        // its write completes, and a failed assertion must not leave it
        // blocked. Nothing here may panic — this runs during unwinding too.
        self.writer.release_all_flushes();
        let _ = self.stall_outcome();
        if let Some(handler) = self.handler.take()
            && let Ok(handler) = Arc::try_unwrap(handler)
        {
            handler.close();
        }
    }
}

/// Fixture: a stalled handler whose queue holds `capacity` slots.
#[fixture]
pub(super) fn stalled_handler(#[default(4)] capacity: usize) -> StalledFlushHandler {
    StalledFlushHandler::new(capacity)
}
