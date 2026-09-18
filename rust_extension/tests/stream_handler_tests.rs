//! Behavioural tests for the stream handler and its worker thread lifecycle.

use std::io::{self, Write};
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{
    DefaultFormatter, FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, FemtoStreamHandler,
    HandlerError, StreamHandlerConfig,
    rate_limited_warner::{Clock, RateLimitedWarner},
};
use rstest::*;
use serial_test::serial;

#[path = "test_utils/mod.rs"]
mod test_utils;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use test_utils::captured_log::captured_log;
use test_utils::fixtures::handler_tuple;
use test_utils::handle_expect::HandleExpect;
use test_utils::shared_buffer::std::read_output;
use test_utils::std::SharedBuf;

#[derive(Clone)]
struct BlockingBuf {
    buf: Arc<Mutex<Vec<u8>>>,
    barrier: Arc<Barrier>,
}

impl Write for BlockingBuf {
    // This test double is driven from the handler's worker thread, so a
    // poisoned lock must be recovered rather than turned into a second panic
    // that would obscure the original failure.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Block until the test thread releases the barrier
        self.barrier.wait();
        self.buf
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush()
    }
}

#[derive(Default)]
struct FlushFailingBuf {
    buf: Vec<u8>,
}

impl Write for FlushFailingBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("flush failed"))
    }
}

#[rstest]
fn stream_handler_writes_to_buffer(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hello"));
    drop(handler); // ensure thread completes

    assert_eq!(read_output(&buffer), "core [INFO] hello\n");
}

#[rstest]
fn stream_handler_multiple_records(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Warn, "second"));
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Error, "third"));
    drop(handler);

    let output = read_output(&buffer);
    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    );
}

#[rstest]
fn stream_handler_flush(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "one"));
    assert!(handler.flush());
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "two"));
    drop(handler);

    assert_eq!(read_output(&buffer), "core [INFO] one\ncore [INFO] two\n");
}

#[test]
fn stream_handler_flush_returns_false_on_worker_flush_error() {
    let handler = FemtoStreamHandler::new(FlushFailingBuf::default(), DefaultFormatter);
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "one"));
    assert!(!handler.flush());
}

#[rstest]
fn stream_handler_close_flushes_pending(
    #[from(handler_tuple)] (buffer, mut handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "close"));
    handler.close();

    assert_eq!(read_output(&buffer), "core [INFO] close\n");
}

#[rstest]
fn stream_handler_flush_after_close(
    #[from(handler_tuple)] (_buffer, mut handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.close();
    assert!(!handler.flush());
}

#[rstest]
fn stream_handler_concurrent_usage(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    let handler = Arc::new(handler);

    let mut handles = vec![];
    for i in 0..10 {
        let h = Arc::clone(&handler);
        handles.push(thread::spawn(move || {
            h.expect_handle(FemtoLogRecord::new(
                "core",
                FemtoLevel::Info,
                &format!("msg{}", i),
            ));
        }));
    }
    for h in handles {
        h.join().expect("producer thread panicked");
    }
    drop(handler);

    let output = read_output(&buffer);
    for i in 0..10 {
        assert!(output.contains(&format!("core [INFO] msg{}", i)));
    }
}

#[rstest]
fn stream_handler_trait_object_usage(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    let handler: Box<dyn FemtoHandlerTrait> = Box::new(handler);
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "trait"));
    drop(handler);

    assert_eq!(read_output(&buffer), "core [INFO] trait\n");
}

#[rstest]
fn stream_handler_poisoned_mutex(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    // Poison the mutex by panicking while holding the lock
    let test_buffer = Arc::clone(&buffer);
    {
        let b = Arc::clone(&buffer);
        let _ = std::panic::catch_unwind(move || {
            let _guard = b.lock().expect("buffer mutex should not yet be poisoned");
            panic!("poison");
        });
    }

    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "ok"));
    drop(handler);

    // The buffer should remain poisoned; handler must not panic
    assert!(
        test_buffer.lock().is_err(),
        "Buffer mutex should remain poisoned",
    );
}

#[rstest]
/// Ensure dropping a handler with a slow writer doesn't block
/// indefinitely. The worker thread should exit after the one
/// second timeout even if the stream flush takes longer. The test
/// allows a 500ms buffer to accommodate scheduling jitter.
fn stream_handler_drop_timeout() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let barrier = Arc::new(Barrier::new(2));
    let handler = FemtoStreamHandler::new(
        BlockingBuf {
            buf: Arc::clone(&buffer),
            barrier: Arc::clone(&barrier),
        },
        DefaultFormatter,
    );
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "slow"));
    let start = Instant::now();
    drop(handler);
    assert!(start.elapsed() < Duration::from_millis(1500));
    // The extra half second gives the test leeway for scheduler jitter
    // while still proving the drop doesn't hang indefinitely.
    // Allow the worker thread to finish
    barrier.wait();
}

/// Scenario: one caller emits a record and ends without reading it, then
/// another asks for the logger.
///
/// Invariant: the record the first caller left is gone. `logtest::Logger`
/// holds no state; the records live in a queue inside `logtest` that outlives
/// the test that produced them, so a test asserting on an exact count would
/// otherwise read an earlier test's output as its own. A test that panics part
/// way through leaves exactly this residue.
///
/// Written as one test rather than two so it proves the drain without
/// depending on the order the harness chooses.
///
/// The assertion names the marker rather than asking whether anything is left.
/// The capture is process-wide and the mutex scopes only the callers that take
/// it, so "the queue is empty" is a claim about every record the binary emits,
/// while "the record I left is gone" is a claim about the drain. Only the
/// second is this test's business.
///
/// Nothing emits alongside it in the lane that runs it: `heavy-tests` passes
/// `-- --ignored`, which runs these three tests and no others, and they are
/// `#[serial]`. Under `--include-ignored` the ordinary tests in this file run
/// too and do emit, which is the measurement below. Naming the marker is what
/// makes the test mean the same thing in both.
///
/// `#[ignore]` for the same reason its two neighbours carry it. Installing
/// the capture logger makes every record emitted anywhere in this process its
/// own, and the ordinary tests in this file drop records and warn while they
/// do it. Measured: running this file with `--include-ignored` gives the
/// rate-limiting test three warnings where it requires two, because those
/// tests are not serialized against it and emit while it counts. The three
/// logger tests therefore share a lane in which nothing else runs.
///
/// Mutation proof (2026-09-16), each applied alone against
/// `--no-default-features -- --ignored`, and reverted:
///
/// - removing the drain from `captured_log` fails this test, which reports the
///   record it was handed back;
/// - emitting one unrelated record between the drain and the assertion, which
///   is what a caller outside the mutex does, passes. The assertion this
///   replaced, that the queue was empty, fails on that same record, which is
///   the flake it was carrying.
#[rstest]
#[serial]
#[ignore]
fn captured_log_hands_out_no_records_an_earlier_caller_left() {
    const MARKER: &str = "left behind by a caller that never read it";
    {
        let _logger = captured_log();
        log::warn!("{MARKER}");
    }
    let mut logger = captured_log();
    let stale = logger
        .by_ref()
        .find(|record| record.args().to_string().contains(MARKER));
    assert!(
        stale.is_none(),
        "captured_log must drain the records an earlier caller left behind, \
         it handed back {stale:?}"
    );
}

#[rstest]
#[serial]
#[ignore]
fn stream_handler_reports_dropped_records() {
    let mut logger = captured_log();
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = FemtoStreamHandler::with_capacity_timeout(
        SharedBuf::new(Arc::clone(&buffer)),
        DefaultFormatter,
        1,
        Duration::from_millis(50),
    );

    let _ = handler.handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
    let _ = handler.handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"));
    assert!(handler.flush());

    let warnings: Vec<_> = logger
        .by_ref()
        .filter(|r| r.level() == log::Level::Warn)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|r| r.args().to_string().contains("1 log records dropped"))
    );
}

/// The text every dropped-record warning carries.
///
/// The capture logger is process-wide, so counting warnings by level alone
/// would count any other warning the binary emits as a dropped record. This
/// names the one message under test.
const DROPPED_RECORD_WARNING: &str = "log records dropped in the last interval";

/// A clock the test moves by hand.
///
/// [`RateLimitedWarner`] asks its clock for milliseconds and compares the
/// answer with the interval, so supplying the time is what makes the interval
/// boundary a decision this test makes rather than one the scheduler makes.
struct TestClock {
    now_ms: AtomicU64,
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }
}

impl TestClock {
    fn new() -> Self {
        Self {
            now_ms: AtomicU64::new(0),
        }
    }

    fn advance(&self, milliseconds: u64) {
        self.now_ms.fetch_add(milliseconds, Ordering::Relaxed);
    }
}

/// A writer that stops inside `write` until the test lets it go.
///
/// The worker dequeues a record and then writes it, so holding it inside
/// `write` is what holds the queue full: the handler's channel has room for
/// one command and the worker is not coming back for it. Without this the
/// worker drains between calls and whether a record is dropped depends on
/// which thread ran, which is the defect this fixture removes.
#[derive(Clone)]
struct GatedBuf {
    /// Held by the test while the worker must stay inside `write`.
    gate: Arc<Mutex<()>>,
    /// Signals that the worker has dequeued a record and reached the gate.
    entered: std::sync::mpsc::Sender<()>,
}

impl Write for GatedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Report arrival before blocking, so the test knows the queue is empty
        // and the worker is parked rather than merely slow.
        let _ = self.entered.send(());
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Scenario: records are dropped repeatedly, within one warning interval and
/// across the boundary between two.
///
/// Invariant: the handler warns on the first drop, stays silent for every
/// further drop inside the same interval, and warns again on the first drop
/// after the interval elapses. Two warnings for four drops.
///
/// Every input is controlled rather than raced for:
///
/// - the drops are real and asserted. The worker is parked inside `write`
///   with the queue full, so each of the four calls returns
///   `HandlerError::QueueFull` and the test says so. The previous shape sent
///   records into a running handler and hoped the worker had not drained
///   them, which made the warning count depend on thread scheduling;
/// - the interval boundary is crossed by moving a clock, not by sleeping.
///   The previous shape slept 60 milliseconds against a 50-millisecond
///   interval, so a loaded host could suspend the thread between the sleep
///   and the next call and a tick either side of the boundary decided the
///   count.
///
/// Warnings are matched by their text rather than by level. The capture logger
/// is installed once for the whole process, so a warning from anywhere else in
/// the binary would otherwise be counted as a dropped record.
///
/// Mutation proof (2026-09-18), each applied alone against
/// `--no-default-features --test stream_handler_tests -- --ignored`, and
/// reverted:
///
/// - removing the `clock.advance` before the fourth drop fails with `left: 1,
///   right: 2`. Without it the fourth drop falls inside the first interval and
///   is suppressed, so the test discriminates the boundary rather than merely
///   counting drops;
/// - making `warn_if_due` emit whatever the clock says fails with `left: 4,
///   right: 2`, which is the suppression half;
/// - releasing the gate before the drops, with a sleep long enough for the
///   worker to drain, fails on the first `handle`: `"first drop" should have
///   been dropped by a full queue, saw Ok(())`. That is exactly the
///   scheduling dependence the old test carried silently, and it now fails
///   loudly rather than changing the warning count.
///
/// Narrowness control, which passes: advancing the clock by seven intervals
/// and three milliseconds rather than exactly one interval. The contract is
/// about crossing the boundary, not about the size of the step, and one that
/// pinned the step would refuse a correct test written differently.
#[rstest]
#[serial]
#[ignore]
fn stream_handler_rate_limits_warnings() {
    const INTERVAL: Duration = Duration::from_millis(50);

    let mut logger = captured_log();
    let clock = Arc::new(TestClock::new());
    let gate = Arc::new(Mutex::new(()));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();

    let held = gate.lock().unwrap_or_else(PoisonError::into_inner);
    let handler = FemtoStreamHandler::with_test_config(
        GatedBuf {
            gate: Arc::clone(&gate),
            entered: entered_tx,
        },
        DefaultFormatter,
        StreamHandlerConfig::default()
            .with_capacity(1)
            .with_timeout(INTERVAL)
            .with_warner(RateLimitedWarner::with_clock(
                INTERVAL,
                Arc::clone(&clock) as Arc<dyn Clock>,
            )),
    );

    // The worker dequeues this one and parks inside `write`, leaving the
    // channel empty.
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "parking"));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker should reach the gate");

    // This one fills the channel's single slot, and every record after it is
    // dropped for as long as the gate is held.
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "queued"));

    let drop_record = |message: &str| {
        let outcome = handler.handle(FemtoLogRecord::new("core", FemtoLevel::Info, message));
        assert!(
            matches!(outcome, Err(HandlerError::QueueFull)),
            "{message:?} should have been dropped by a full queue, saw {outcome:?}"
        );
    };

    // Clock at 0: the first drop ever seen always warns.
    drop_record("first drop");
    // Still at 0, inside the interval: suppressed.
    drop_record("second drop");
    drop_record("third drop");
    // On the boundary: warns again.
    clock.advance(INTERVAL.as_millis() as u64);
    drop_record("fourth drop");

    let warnings: Vec<_> = logger
        .by_ref()
        .filter(|record| {
            record.level() == log::Level::Warn
                && record.args().to_string().contains(DROPPED_RECORD_WARNING)
        })
        .collect();
    assert_eq!(
        warnings.len(),
        2,
        "four drops either side of one interval boundary should warn twice, saw {warnings:?}"
    );

    // Let the worker finish so the handler's own shutdown does not time out.
    drop(held);
}
