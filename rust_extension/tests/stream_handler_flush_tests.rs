//! Behavioural tests for the stream handler's flush deadline.
//!
//! `flush()` has two waits: queueing its command on a queue that may be full,
//! and waiting for the worker's acknowledgement. Each writer below blocks the
//! worker on a signal the test controls, so the resulting bounds are observed
//! rather than inferred from scheduling.

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use _femtologging_rs::{
    DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler, stream_waiter_count,
};

#[path = "test_utils/handle_expect.rs"]
mod handle_expect;
use handle_expect::HandleExpect;

/// Upper bound on how long a test writer or waiter waits for its signal.
///
/// The bound only stops a broken test from hanging the worker forever; a
/// passing test delivers the signal long before it expires.
const RELEASE_WAIT: Duration = Duration::from_secs(10);

/// Deadline both stages of one flush share in the shared-deadline test.
const SHARED_TIMEOUT: Duration = Duration::from_secs(4);

/// How long the queue stays full while the flush is blocked queueing.
///
/// Long enough that a fresh timeout for the acknowledgement would be
/// unmistakable, short enough to leave most of a deadline unspent.
const ENQUEUE_HOLD: Duration = Duration::from_secs(2);

/// How often a test re-reads the waiter counter while a call is blocking.
const WAIT_POLL: Duration = Duration::from_millis(2);

/// Serialises the tests in this binary.
///
/// The waiter counter is process-wide and shared by every handler, so two
/// tests waiting at once would each see the other's call and could proceed
/// before their own flusher had reached its wait.
static WAIT_COUNTER_LOCK: Mutex<()> = Mutex::new(());

/// Take the waiter-counter lock, treating a poisoned lock as free.
fn lock_wait_counter() -> MutexGuard<'static, ()> {
    WAIT_COUNTER_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wait until some stream-handler call in this process is blocked in a wait.
///
/// The counter rises inside `flush` just before its first blocking send, so a
/// successful wait proves the call has reached the boundary the test holds it
/// at, rather than merely having been scheduled.
fn wait_until_a_call_is_blocked(context: &str) {
    let deadline = Instant::now() + RELEASE_WAIT;
    while stream_waiter_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "{context}: no call reached its wait within {RELEASE_WAIT:?}"
        );
        thread::sleep(WAIT_POLL);
    }
}

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

/// `flush` reports failure when the worker never drains the queue.
#[test]
fn stream_handler_flush_returns_false_when_the_queue_stays_full() {
    const FLUSH_TIMEOUT: Duration = Duration::from_millis(100);

    let _serial = lock_wait_counter();
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

/// A handler whose worker blocks in its leading flushes, plus its signals.
struct GatedHandler {
    /// Handler under test.
    handler: Arc<FemtoStreamHandler>,
    /// Fires each time the worker enters a gated flush.
    entered: Receiver<()>,
    /// Released once per `release_worker` call.
    release: Sender<()>,
}

impl GatedHandler {
    /// Build a handler whose worker blocks in its first `gates` flushes.
    ///
    /// Three gates cover the first record's flush, the second record's flush,
    /// and the flush command's flush; the last can only be reached if the
    /// command really was queued.
    fn with_gates(gates: u8, flush_timeout: Duration) -> Self {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let handler = FemtoStreamHandler::with_capacity_timeout(
            GatedFlushBuf {
                entered: entered_tx,
                releases: release_rx,
                gates,
            },
            DefaultFormatter,
            1,
            flush_timeout,
        );
        Self {
            handler: Arc::new(handler),
            entered: entered_rx,
            release: release_tx,
        }
    }

    /// Wait until the worker blocks in its next gated flush.
    fn wait_for_worker_block(&self, context: &str) {
        // The repository's lint rejects a panicking `unwrap_or_else`; naming
        // the context in a panic is the point, so report it explicitly.
        if let Err(err) = self.entered.recv_timeout(RELEASE_WAIT) {
            panic!("{context}: {err}");
        }
    }

    /// Release the worker's current gated flush.
    fn release_worker(&self) {
        self.release
            .send(())
            .expect("release the worker's blocked flush");
    }

    /// Block the worker in its first flush and fill the only queue slot behind it.
    fn block_worker_with_full_queue(&self) {
        self.handler
            .expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
        self.wait_for_worker_block("worker should block in its first flush");
        // Fill the only queue slot while the worker is blocked, so the flush
        // command has to wait for space.
        self.handler
            .expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"));
    }
}

/// Outcome of one flush call, measured inside the thread that performed it.
struct FlushOutcome {
    /// What `flush()` returned.
    flushed: bool,
    /// How long the call took, measured in the flusher.
    waited: Duration,
}

/// A running flush call whose timing the test measures.
struct Flusher {
    /// Thread running the call.
    handle: JoinHandle<()>,
    /// Delivers the measured outcome.
    outcome: Receiver<FlushOutcome>,
    /// Signals that the thread is about to call `flush()`.
    entering: Receiver<()>,
}

impl Flusher {
    /// Spawn a thread that calls `flush()` on `handler` immediately.
    ///
    /// Entry is reported from inside the flusher, immediately before the call
    /// whose deadline is under test. Timing the hold from the spawn instead
    /// would let a delayed thread start late and spend more of its deadline
    /// after the release.
    fn spawn(handler: Arc<FemtoStreamHandler>) -> Self {
        let (entering_tx, entering_rx) = mpsc::channel();
        let (outcome_tx, outcome_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            entering_tx
                .send(())
                .expect("signal that the flusher is entering its wait");
            let started = Instant::now();
            let flushed = handler.flush();
            // Measure the wait in the thread that performs it, so a delayed
            // test thread cannot inflate the result into a second timeout.
            let _ = outcome_tx.send(FlushOutcome {
                flushed,
                waited: started.elapsed(),
            });
        });
        Self {
            handle,
            outcome: outcome_rx,
            entering: entering_rx,
        }
    }

    /// Wait until the flusher reports that it is about to call `flush()`.
    fn wait_until_entering(&self) {
        self.entering
            .recv_timeout(RELEASE_WAIT)
            .expect("flusher should enter its wait before the queue is held");
    }

    /// Wait for the measured outcome and join the flusher.
    fn finish(self) -> FlushOutcome {
        let outcome = self
            .outcome
            .recv_timeout(RELEASE_WAIT)
            .expect("flush should report within the shared deadline");
        self.handle.join().expect("flusher thread panicked");
        outcome
    }
}

/// Assert that one flush waited out the hold but not a second timeout.
fn assert_shared_deadline(outcome: &FlushOutcome) {
    assert!(
        !outcome.flushed,
        "flush() must fail when the worker never acknowledges"
    );
    // The queue stayed full for the whole hold, so a flush that returned
    // sooner never waited to enqueue, and one that took a fresh timeout after
    // enqueueing would exceed the upper bound below.
    assert!(
        outcome.waited >= ENQUEUE_HOLD,
        "flush() returned after {:?}, without waiting out the {ENQUEUE_HOLD:?} hold",
        outcome.waited
    );
    assert!(
        outcome.waited < SHARED_TIMEOUT + ENQUEUE_HOLD / 2,
        concat!(
            "flush() waited {:?}; ",
            "queueing and acknowledgement did not share one deadline"
        ),
        outcome.waited
    );
}

/// A flush that queues late must still expire at its original deadline.
///
/// The queue frees up only after most of the timeout has passed, and the
/// worker then blocks again without acknowledging. Only the remainder of the
/// shared deadline may be spent waiting, so a second full timeout is
/// distinguishable from the expected short wait.
///
/// The test waits for the flusher's own entry signal and then for the waiter
/// counter to rise, so the hold below is provably spent inside the queueing
/// send rather than in a thread that has not started yet. After the deadline
/// expires, the worker is released and must then consume the flush command:
/// entering its third flush proves the command really was queued, which the
/// timing alone cannot show.
#[test]
fn stream_handler_flush_shares_one_deadline_between_queueing_and_acknowledgement() {
    let _serial = lock_wait_counter();
    let gated = GatedHandler::with_gates(3, SHARED_TIMEOUT);
    gated.block_worker_with_full_queue();

    let outcome = flush_while_the_queue_stays_full(&gated);
    assert_shared_deadline(&outcome);

    prove_the_flush_command_was_queued(&gated);
    gated.handler.close();
}

/// Start a flush against a full queue, hold it, then release the worker once.
///
/// The hold is only begun after the flusher's own entry signal and the waiter
/// counter both confirm it is blocked on the full queue, so the measured wait
/// belongs to the queueing send whatever the scheduler did. The worker is
/// released after the hold and must then block in its second flush, which lets
/// the flush command enqueue while no acknowledgement can follow.
fn flush_while_the_queue_stays_full(gated: &GatedHandler) -> FlushOutcome {
    let flusher = Flusher::spawn(Arc::clone(&gated.handler));
    flusher.wait_until_entering();
    wait_until_a_call_is_blocked("flush should block while queueing its command");

    thread::sleep(ENQUEUE_HOLD);
    gated.release_worker();
    gated.wait_for_worker_block("worker should block in its second flush");
    flusher.finish()
}

/// Release the worker past its second flush and require it to reach a third.
///
/// The worker was blocked when the deadline expired, so the flush can only
/// have reached its acknowledgement wait if the command was queued behind the
/// blocked record. Entering the third gated flush is proof of that enqueue.
fn prove_the_flush_command_was_queued(gated: &GatedHandler) {
    gated.release_worker();
    gated.wait_for_worker_block("the flush command was never queued for the worker");
    gated.release_worker();
}
