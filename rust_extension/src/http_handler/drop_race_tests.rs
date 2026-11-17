//! A controlled collision between two drop reports through the production
//! helper.
//!
//! `drop_reporting_tests.rs` races two real threads and cannot decide which of
//! them wins. Here the test parks the losing caller inside the warner's
//! report-time sample, so the winner is chosen by the test and the message it
//! produces can only be its own cause and count. The callers go through
//! `warn_drops_naming_cause`, so this pins the helper the handler actually uses
//! rather than a closure retyped beside it.
//!
//! Included with `#[path]` from `tests.rs`, like `drop_reporting_tests.rs`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::http_handler::queue::{flush_drops, warn_drops_naming_cause};
use crate::rate_limited_warner::{Clock, RateLimitedWarner};

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

    // Open the interval, then leave two drops suppressed inside it under a
    // cause neither racer reports, so a misattributed message is visible.
    warn_drops_naming_cause(&warner, "queue full");
    warn_drops_naming_cause(&warner, "permanent error");
    warn_drops_naming_cause(&warner, "permanent error");
    let opening: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        opening,
        vec![
            concat!(
                "FemtoHTTPHandler dropped 1 records since the last report; ",
                "triggering cause: queue full"
            )
            .to_owned()
        ],
        "only the first report should escape the interval"
    );
    clock.now_ms.store(1000, Ordering::Relaxed);

    // The parked caller samples the clock once for its due pre-check and once
    // for its claim, so its report-time sample is two indices later.
    clock
        .park_at
        .store(clock.samples.load(Ordering::Relaxed) + 2, Ordering::Relaxed);

    thread::scope(|scope| {
        scope.spawn(|| warn_drops_naming_cause(&warner, "queue full"));
        if parked_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            let _ = resume_tx.send(());
            panic!("the first caller must park after claiming the interval");
        }

        // The parked caller's batch is out of the counter, so these two drops
        // are all that is pending; at the unchanged clock they stay suppressed.
        warn_drops_naming_cause(&warner, "permanent error");
        warn_drops_naming_cause(&warner, "permanent error");

        // A whole interval after the parked caller's claim, the next report is
        // due: it claims the interval and supersedes the parked caller.
        clock.now_ms.store(2000, Ordering::Relaxed);
        warn_drops_naming_cause(&warner, "worker disconnected");
        resume_tx
            .send(())
            .expect("the parked caller receives its release");
    });

    // Another serial test waiting for its turn logs through the shared logger,
    // so only this handler's own reports are compared.
    let messages: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .filter(|message| message.starts_with("FemtoHTTPHandler"))
        .collect();
    assert_eq!(
        messages,
        vec![
            concat!(
                "FemtoHTTPHandler dropped 3 records since the last report; ",
                "triggering cause: worker disconnected"
            )
            .to_owned()
        ],
        "only the caller that published the interval reports, naming its own cause"
    );
    assert_eq!(
        warner.pending_drops(),
        3,
        "the superseded caller's batch must stay pending"
    );

    flush_drops(&warner);
    let flushed: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .filter(|message| message.starts_with("FemtoHTTPHandler"))
        .collect();
    assert_eq!(
        flushed,
        vec!["FemtoHTTPHandler dropped 3 records since the last report".to_owned()],
        "the superseded batch is still reportable, and the flush invents no cause"
    );
    assert_eq!(warner.pending_drops(), 0);
}
