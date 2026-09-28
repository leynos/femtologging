//! The stream handler's logging behaviour, captured through the `log` facade.
//!
//! These three tests install the process-wide capture logger, so every record
//! emitted anywhere in their binary is theirs. They live in a binary of their
//! own, apart from `stream_handler_tests.rs`, whose ordinary tests drop records
//! and warn while they run; they are also `#[ignore]` and `#[serial]`, so the
//! heavy lane's `-- --ignored` runs them together with nothing else.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use _femtologging_rs::{
    DefaultFormatter, FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, FemtoStreamHandler,
    HandlerError, StreamHandlerConfig,
    rate_limited_warner::{Clock, RateLimitedWarner},
};
use rstest::*;
use serial_test::serial;

#[path = "test_utils/captured_log.rs"]
mod captured_log;
#[path = "test_utils/handle_expect.rs"]
mod handle_expect;

use captured_log::captured_log;
use handle_expect::HandleExpect;

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
/// Nothing emits alongside it: this binary holds only the three capture-logger
/// tests, `heavy-tests` runs them with `-- --ignored`, and they are
/// `#[serial]`. Naming the marker keeps the test meaning the same thing if an
/// emitting test ever joins the binary.
///
/// `#[ignore]` for the same reason its two neighbours carry it. Installing
/// the capture logger makes every record emitted anywhere in this process its
/// own. When these tests shared `stream_handler_tests.rs`, whose ordinary
/// tests drop records and warn, running that file with `--include-ignored`
/// gave the rate-limiting test three warnings where it requires two. The
/// three tests now have a binary of their own, and keep a lane in which
/// nothing else runs.
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

/// Scenario: a record arrives while the handler's one-slot queue is full.
///
/// Invariant: the record is refused as `QueueFull` and the handler reports the
/// drop. The worker is held inside `write` by [`GatedBuf`], so the second record
/// occupies the queue and the third is certain to be refused. With two records
/// and a free worker the test depended on the worker not draining in between,
/// and on a busy runner it sometimes did.
#[rstest]
#[serial]
#[ignore]
fn stream_handler_reports_dropped_records() {
    let mut logger = captured_log();
    let gate = Arc::new(Mutex::new(()));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();

    let held = gate.lock().unwrap_or_else(PoisonError::into_inner);
    let handler = FemtoStreamHandler::with_capacity_timeout(
        GatedBuf {
            gate: Arc::clone(&gate),
            entered: entered_tx,
        },
        DefaultFormatter,
        1,
        Duration::from_secs(5),
    );

    // The worker dequeues this one and parks inside `write`.
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "first"));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker should reach the gate");
    // This one fills the channel's single slot, so the next is refused.
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "second"));
    let refused = handler.handle(FemtoLogRecord::new("core", FemtoLevel::Info, "third"));

    drop(held);
    assert!(handler.flush());

    assert_eq!(refused, Err(HandlerError::QueueFull));
    let warnings: Vec<_> = logger
        .by_ref()
        .filter(|r| r.level() == log::Level::Warn)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|r| r.args().to_string().contains("1 log records dropped")),
        "the refused record must be reported as dropped, saw {warnings:?}"
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
