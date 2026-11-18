//! A controlled collision between two drop reports through the production
//! helper.
//!
//! `drop_reporting_tests.rs` races two real threads and cannot decide which of
//! them wins. Here the test parks the losing caller inside the warner's
//! report-time sample, so the winner is chosen by the test and the message it
//! produces can only be its own cause and count. The callers go through
//! `warn_drops_naming_cause`, so this pins the helper the socket handler
//! actually uses rather than a closure retyped beside it.
//!
//! The test logger is process-global, and the rest of this binary runs in
//! parallel, so warnings from other tests land in the same buffer while the
//! losing caller is parked for up to five seconds. Each cause this test reports
//! therefore carries a marker that no other test uses, and the assertions count
//! exact messages instead of comparing the whole buffer: a neighbour's report
//! is ignored, while a second report from this test still fails.
//!
//! Included with `#[path]` from `tests.rs`, like `drop_reporting_tests.rs`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::rate_limited_warner::{Clock, RateLimitedWarner};
use crate::socket_handler::warning::{flush_drops_naming_cause, warn_drops_naming_cause};

/// The cause the racers report, used when the winner must not be identified.
const WINNER_CAUSE: &str = "write error (superseded winner)";

/// A clock that parks one chosen sample until the test releases it.
///
/// The parked caller has claimed the warning interval but has not published a
/// report time, which is exactly the window in which a real caller would be
/// delayed.
struct ParkingClock {
    /// Current test-controlled timestamp in milliseconds.
    now_ms: AtomicU64,
    /// Samples taken so far across all callers.
    samples: AtomicU64,
    /// Index of the sample that parks, or `u64::MAX` while disarmed.
    park_at: AtomicU64,
    /// Signals that the parked caller has reached its sample.
    parked: mpsc::Sender<()>,
    /// Released by the test once the competing caller has reported.
    resume: Mutex<mpsc::Receiver<()>>,
}

impl Clock for ParkingClock {
    fn now_millis(&self) -> u64 {
        let sample = self.samples.fetch_add(1, Ordering::Relaxed);
        if sample == self.park_at.load(Ordering::Relaxed) {
            self.parked.send(()).expect("parked caller receiver");
            self.resume
                .lock()
                .expect("resume lock")
                .recv_timeout(Duration::from_secs(5))
                .expect("the test releases the parked caller");
        }
        self.now_ms.load(Ordering::Relaxed)
    }
}

/// Render the warning naming `cause`, for comparison against captured messages.
fn drop_warning(count: u64, cause: &str) -> String {
    format!(
        concat!(
            "FemtoSocketHandler dropped {count} records since the last report; ",
            "triggering cause: {cause}"
        ),
        count = count,
        cause = cause
    )
}

/// Count the captured messages equal to `expected`.
fn occurrences(messages: &[String], expected: &str) -> usize {
    messages
        .iter()
        .filter(|message| *message == expected)
        .count()
}

/// Drain the shared test logger into the message texts.
fn captured() -> Vec<String> {
    take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect()
}

/// Open the interval, then leave two drops suppressed inside it.
///
/// The suppressed drops use a cause the racers never report, so a misattributed
/// message is visible as a wrong cause rather than as a plausible one.
fn open_interval_with_suppressed_drops(warner: &RateLimitedWarner) {
    warn_drops_naming_cause(warner, "connect failure: refused (superseded race)");
    warn_drops_naming_cause(warner, "serialization failure (superseded race)");
    warn_drops_naming_cause(warner, "serialization failure (superseded race)");
    let opening = captured();
    assert_eq!(
        occurrences(
            &opening,
            &drop_warning(1, "connect failure: refused (superseded race)")
        ),
        1,
        "only the first report should escape the interval: {opening:?}"
    );
}

/// Park a loser after its claim, let a later caller report, then release it.
///
/// The parked caller samples the clock once for its due pre-check and once for
/// its claim, so its report-time sample is two indices after the caller starts.
fn run_superseded_race(
    warner: &RateLimitedWarner,
    clock: &ParkingClock,
    parked_rx: mpsc::Receiver<()>,
    resume_tx: mpsc::Sender<()>,
) {
    clock
        .park_at
        .store(clock.samples.load(Ordering::Relaxed) + 2, Ordering::Relaxed);
    thread::scope(|scope| {
        scope.spawn(|| {
            warn_drops_naming_cause(warner, "connect failure: refused (superseded loser)");
        });
        if parked_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            let _ = resume_tx.send(());
            panic!("the first caller must park after claiming the interval");
        }

        // The parked caller's batch is out of the counter, so these two drops
        // are all that is pending; at the unchanged clock they stay suppressed.
        warn_drops_naming_cause(warner, "serialization failure (superseded race)");
        warn_drops_naming_cause(warner, "serialization failure (superseded race)");

        // A whole interval after the parked caller's claim, the next report is
        // due: it claims the interval and supersedes the parked caller.
        clock.now_ms.store(2000, Ordering::Relaxed);
        warn_drops_naming_cause(warner, WINNER_CAUSE);
        resume_tx
            .send(())
            .expect("the parked caller receives its release");
    });
}

/// Assert only the winner's own count and cause escaped.
fn assert_only_the_winner_reported(warner: &RateLimitedWarner, messages: &[String]) {
    assert_eq!(
        occurrences(messages, &drop_warning(3, WINNER_CAUSE)),
        1,
        "only the caller that published the interval reports, naming its own \
         cause: {messages:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| message.contains("superseded loser")),
        "the superseded caller must not report at all: {messages:?}"
    );
    assert_eq!(
        warner.pending_drops(),
        3,
        "the superseded caller's batch must stay pending"
    );
}

/// Assert a naming flush can still report the superseded batch.
///
/// The flush is told a cause too, so its message is attributable as well; the
/// cause-neutral flush wording is covered by `drop_reporting_tests.rs`.
fn assert_flush_reports_the_superseded_batch(warner: &RateLimitedWarner) {
    flush_drops_naming_cause(warner, "superseded flush (race)");
    let flushed = captured();
    assert_eq!(
        occurrences(&flushed, &drop_warning(3, "superseded flush (race)")),
        1,
        "the superseded batch is still reportable: {flushed:?}"
    );
    assert_eq!(warner.pending_drops(), 0);
}

/// A report superseded while it was publishing must not relabel the winner.
///
/// Scenario: one caller claims the expired interval and parks before publishing
/// its report time, a second caller claims and reports meanwhile, and the first
/// is then released. Invariant: only the second caller's report escapes, and it
/// names its own cause with its own count. The first caller's batch stays
/// pending for the next report rather than joining a warning it did not make.
#[test]
#[serial_test::serial]
fn a_superseded_report_does_not_relabel_the_winners_cause() {
    install_test_logger();
    let (parked_tx, parked_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let clock = Arc::new(ParkingClock {
        now_ms: AtomicU64::new(0),
        samples: AtomicU64::new(0),
        park_at: AtomicU64::new(u64::MAX),
        parked: parked_tx,
        resume: Mutex::new(resume_rx),
    });
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone());

    open_interval_with_suppressed_drops(&warner);
    clock.now_ms.store(1000, Ordering::Relaxed);
    run_superseded_race(&warner, &clock, parked_rx, resume_tx);

    let messages = captured();
    assert_only_the_winner_reported(&warner, &messages);
    assert_flush_reports_the_superseded_batch(&warner);
}
