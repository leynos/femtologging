//! Regression tests for file handler flush acknowledgement semantics.
//!
//! These tests ensure each flush call waits for its own worker
//! acknowledgement rather than reusing a stale ack from an earlier timed-out
//! flush.

use super::test_support::impl_unsupported_seek;
use super::*;
use std::io::{self, ErrorKind, Write};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::handler::FlushContext;
use crate::monotonic_clock::SharedClock;
use crate::test_utils::flush_budget::SharedScriptedClock;

/// Cloneable writer whose flush calls wait for explicit test release.
#[derive(Clone)]
struct BlockingFlushWriter {
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
    fn release_all_flushes(&self) {
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
    let Ok(handler) = Arc::try_unwrap(handler) else {
        panic!("flush test must release all handler references before shutdown");
    };
    handler.close();
}

/// Build a file handler whose worker blocks in the writer's flush.
///
/// The first flush occupies the worker, so anything queued behind it cannot be
/// acknowledged, and the returned receiver reports when the worker reaches the
/// writer.
fn stalled_file_handler(
    capacity: usize,
) -> (
    Arc<FemtoFileHandler>,
    BlockingFlushWriter,
    thread::JoinHandle<()>,
    mpsc::Receiver<bool>,
    mpsc::Receiver<usize>,
) {
    let (started_tx, started_rx) = mpsc::channel();
    let writer = BlockingFlushWriter::new(started_tx);
    let mut config = TestConfig::new(writer.clone(), DefaultFormatter);
    config.capacity = capacity;
    config.flush_interval = 16;
    config.overflow_policy = OverflowPolicy::Block;

    let handler = Arc::new(FemtoFileHandler::with_writer_for_test(config));
    let stalling = Arc::clone(&handler);
    let (stall_done_tx, stall_done_rx) = mpsc::channel();
    let stall = thread::spawn(move || {
        stall_done_tx
            .send(stalling.flush())
            .expect("stall result receiver must stay connected");
    });
    (handler, writer, stall, stall_done_rx, started_rx)
}

/// The supplied deadline, not the handler's one-second timeout, bounds an
/// acknowledgement the stalled worker never sends.
#[test]
fn flush_deadline_bounds_a_stalled_acknowledgement() {
    let (handler, writer, stall, stall_done_rx, started_rx) = stalled_file_handler(4);
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the stalling flush must reach the writer"),
        1
    );

    // Admission has the whole budget; the acknowledgement has none.
    let start = Instant::now();
    let deadline = start + Duration::from_secs(30);
    let mut samples = 0_u8;
    let mut now = || {
        samples += 1;
        // The cap and admission checks see the start, so the enqueue gets the
        // whole budget. Only the acknowledgement sample sees it spent.
        if samples <= 2 {
            start
        } else {
            start + Duration::from_secs(31)
        }
    };
    let began = Instant::now();
    let confirmed = handler.flush_by_at(deadline, &mut now);

    assert!(
        !confirmed,
        "a worker blocked inside the writer cannot acknowledge this flush"
    );
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "the acknowledgement wait must stop at the supplied deadline, not the one-second timeout"
    );
    assert_eq!(
        samples, 3,
        "the cap, queue admission and acknowledgement must each sample the clock"
    );

    writer.release_all_flushes();
    // The stalling flush has its own one-second budget, so whether the released
    // write finishes inside it is a scheduling detail. Only its completion
    // matters here; the assertion above covers the deadline under test.
    let _ = stall_done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the stalling flush must report its outcome");
    stall.join().expect("the stalling flush thread must exit");
    let Ok(handler) = Arc::try_unwrap(handler) else {
        panic!("flush test must release all handler references before shutdown");
    };
    handler.close();
}

/// A caller deadline shorter than the handler's fixed one-second flush timeout
/// bounds the wait through the `flush_until` entry point.
#[test]
fn flush_until_lets_a_shorter_caller_deadline_bound_the_flush() {
    let (handler, writer, stall, stall_done_rx, started_rx) = stalled_file_handler(4);
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the stalling flush must reach the writer"),
        1
    );

    // The caller allows fifty milliseconds; the handler's own budget is one
    // second. The second sample is already past the caller's deadline, so
    // neither queue wait may block.
    let clock = Arc::new(SharedScriptedClock::new(&[0, 100]));
    let injected: SharedClock = clock.clone();
    let context = FlushContext::with_clock(
        clock.started_at(),
        Some(clock.deadline_within(Duration::from_millis(50))),
        injected,
    );

    let began = Instant::now();
    let confirmed = handler.flush_until(&context);
    let elapsed = began.elapsed();

    assert!(
        !confirmed,
        "a worker blocked inside the writer cannot acknowledge this flush"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "the caller's 50 ms deadline must bound the wait, not the one-second timeout"
    );
    assert_eq!(
        clock.samples(),
        3,
        "the cap, queue admission and acknowledgement must each sample the context clock"
    );

    writer.release_all_flushes();
    let _ = stall_done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the stalling flush must report its outcome");
    stall.join().expect("the stalling flush thread must exit");
    let Ok(handler) = Arc::try_unwrap(handler) else {
        panic!("flush test must release all handler references before shutdown");
    };
    handler.close();
}

/// A full queue must refuse the marker once the supplied deadline has no
/// budget left, even though the handler's timeout would still allow a wait.
#[test]
fn flush_deadline_bounds_a_stalled_queue_admission() {
    let (handler, writer, stall, stall_done_rx, started_rx) = stalled_file_handler(1);
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the stalling flush must reach the writer"),
        1
    );

    // The stalled worker cannot drain this record, so it holds the only slot.
    handler
        .handle(FemtoLogRecord::new(
            "flush-deadline",
            FemtoLevel::Info,
            "occupies the queue",
        ))
        .expect("the free slot should accept one record");

    let start = Instant::now();
    let deadline = start + Duration::from_secs(30);
    let mut samples = 0_u8;
    let mut now = || {
        samples += 1;
        start + Duration::from_secs(31)
    };
    let began = Instant::now();
    let confirmed = handler.flush_by_at(deadline, &mut now);

    assert!(!confirmed, "a full queue cannot admit the flush marker");
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "queue admission must stop at the supplied deadline, not wait for room"
    );
    assert_eq!(
        samples, 2,
        "a refused admission must still sample the cap and the admission budget"
    );

    writer.release_all_flushes();
    // As above: the stalling flush's own one-second budget decides its result,
    // which is not the deadline this test asserts on.
    let _ = stall_done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the stalling flush must report its outcome");
    stall.join().expect("the stalling flush thread must exit");
    let Ok(handler) = Arc::try_unwrap(handler) else {
        panic!("flush test must release all handler references before shutdown");
    };
    handler.close();
}
