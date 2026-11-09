//! Behavioural tests for the stream handler's flush deadline.
//!
//! `flush()` has two waits: queueing its command on a queue that may be full,
//! and waiting for the worker's acknowledgement. Each writer below blocks the
//! worker on a signal the test controls, so the resulting bounds are observed
//! rather than inferred from scheduling.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler};

#[path = "test_utils/handle_expect.rs"]
mod handle_expect;
use handle_expect::HandleExpect;

/// Upper bound on how long a test writer waits for its release signal.
///
/// The bound only stops a broken test from hanging the worker forever; a
/// passing test releases the writer long before it expires.
const RELEASE_WAIT: Duration = Duration::from_secs(10);

/// Writer whose first flush blocks until the test releases it.
struct BlockingFlushBuf {
    /// Signals that the worker has reached the first flush.
    entered: Sender<()>,
    /// The first flush waits for this channel; later flushes return at once.
    release: Receiver<()>,
    /// Set once the first flush has been released.
    released: bool,
}

impl Write for BlockingFlushBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.released {
            let _ = self.entered.send(());
            let _ = self.release.recv_timeout(RELEASE_WAIT);
            self.released = true;
        }
        Ok(())
    }
}

#[test]
fn stream_handler_flush_returns_false_when_the_queue_stays_full() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(100);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let handler = FemtoStreamHandler::with_capacity_timeout(
        BlockingFlushBuf {
            entered: entered_tx,
            release: release_rx,
            released: false,
        },
        DefaultFormatter,
        1,
        FLUSH_TIMEOUT,
    );
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "blocking"));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker should reach the blocking write");
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "queued"));

    let started = Instant::now();
    assert!(
        !handler.flush(),
        "flush() must fail when the worker never drains the queue"
    );
    let elapsed = started.elapsed();
    // The enqueue wait alone is bounded by one flush timeout. The generous
    // factor only catches a hang; it does not measure scheduling jitter.
    assert!(
        elapsed < FLUSH_TIMEOUT * 4,
        "flush() waited {elapsed:?}; queueing must be bounded by the flush timeout"
    );

    release_tx.send(()).expect("release the blocked writer");
    handler.close();
}

/// Writer that blocks the worker's leading flushes until the test releases them.
struct GatedFlushBuf {
    /// Signals each time the worker enters a gated flush.
    entered: Sender<()>,
    /// Release signals for the gated flushes, consumed in order.
    releases: Receiver<()>,
    /// How many leading flush calls block; later calls return at once.
    gates: u8,
}

impl Write for GatedFlushBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.gates == 0 {
            return Ok(());
        }
        self.gates -= 1;
        let _ = self.entered.send(());
        let _ = self.releases.recv_timeout(RELEASE_WAIT);
        Ok(())
    }
}

/// A flush that queues late must still expire at its original deadline.
///
/// The queue frees up only after most of the timeout has passed, and the
/// worker then blocks again without acknowledging. Only the remainder of the
/// shared deadline may be spent waiting, so a second full timeout is
/// distinguishable from the expected short wait.
///
/// The flusher reports entry immediately before the call whose deadline is
/// under test and measures its own return, so neither a delayed thread start
/// nor a delayed test thread can distort the measured wait. After the deadline
/// expires, the worker is released and must then consume the flush command:
/// entering its third flush proves the command really was queued, which the
/// timing alone cannot show.
#[test]
fn stream_handler_flush_shares_one_deadline_between_queueing_and_acknowledgement() {
    const FLUSH_TIMEOUT: Duration = Duration::from_secs(4);
    // Long enough that a fresh timeout for the acknowledgement would be
    // unmistakable, short enough to leave most of a deadline unspent.
    const ENQUEUE_HOLD: Duration = Duration::from_secs(2);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let handler = Arc::new(FemtoStreamHandler::with_capacity_timeout(
        GatedFlushBuf {
            entered: entered_tx,
            releases: release_rx,
            // Three gates: the first record's flush, the second record's
            // flush, and the flush command's flush. The last can only be
            // reached if the command was queued.
            gates: 3,
        },
        DefaultFormatter,
        1,
        FLUSH_TIMEOUT,
    ));
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
    entered_rx
        .recv_timeout(RELEASE_WAIT)
        .expect("worker should block in its first flush");
    // Fill the only queue slot while the worker is blocked, so the flush
    // command has to wait for space.
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"));

    let (started_tx, started_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let flusher = {
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            // Report entry from inside the flusher, immediately before the
            // call whose deadline is under test. Timing the hold from the
            // spawn instead would let a delayed thread start late and spend
            // more of its deadline after the release below.
            started_tx
                .send(())
                .expect("signal that the flusher is entering its wait");
            let started = Instant::now();
            let flushed = handler.flush();
            // Measure the wait in the thread that performs it, so a delayed
            // test thread cannot inflate the result into a second timeout.
            let _ = finished_tx.send((flushed, started.elapsed()));
        })
    };
    started_rx
        .recv_timeout(RELEASE_WAIT)
        .expect("flusher should enter its wait before the queue is held");

    thread::sleep(ENQUEUE_HOLD);
    release_tx
        .send(())
        .expect("release the worker's first flush");
    // The worker now takes the queued record, writes it, and blocks in its
    // second flush, so the flush command can enqueue but no ack can follow.
    entered_rx
        .recv_timeout(RELEASE_WAIT)
        .expect("worker should block in its second flush");

    let (flushed, waited) = finished_rx
        .recv_timeout(RELEASE_WAIT)
        .expect("flush should report within the shared deadline");
    assert!(
        !flushed,
        "flush() must fail when the worker never acknowledges"
    );
    // The queue stayed full for the whole hold, so a flush that returned
    // sooner never waited to enqueue, and one that took a fresh timeout after
    // enqueueing would exceed the upper bound below.
    assert!(
        waited >= ENQUEUE_HOLD,
        "flush() returned after {waited:?}, without waiting out the {ENQUEUE_HOLD:?} hold"
    );
    assert!(
        waited < FLUSH_TIMEOUT + ENQUEUE_HOLD / 2,
        concat!(
            "flush() waited {:?}; ",
            "queueing and acknowledgement did not share one deadline"
        ),
        waited
    );

    // The worker was blocked when the deadline expired, so the call can only
    // have reached its acknowledgement wait if the command was queued behind
    // the blocked record. Releasing the second flush lets the worker take that
    // command next; entering the third flush is proof of the enqueue.
    release_tx
        .send(())
        .expect("release the worker's second flush");
    entered_rx
        .recv_timeout(RELEASE_WAIT)
        .expect("the flush command was never queued for the worker");
    release_tx
        .send(())
        .expect("release the flush command's flush");
    flusher.join().expect("flusher thread panicked");
    handler.close();
}
