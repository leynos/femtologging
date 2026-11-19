//! Contracts for what the HTTP handler reports when it drops records.
//!
//! One warner counts drops from several causes, and a suppression can make a
//! count span more than one interval. These tests pin the two consequences: a
//! cumulative warning names only the cause that triggered it, and a report that
//! arrives on flush, close or destruction still accounts for drops the interval
//! was holding back.
//!
//! Included with `#[path]` from `tests.rs`, like `close_tests.rs`, so the
//! fixture and helpers in that file stay in scope.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use crate::handler::FemtoHandlerTrait;
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::http_handler::FemtoHTTPHandler;
use crate::http_handler::config::HTTPHandlerConfig;
use crate::http_handler::queue::{flush_drops, warn_cumulative_drops, warn_drops_naming_cause};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::rate_limited_warner::{Clock, RateLimitedWarner};

/// A clock the test moves by hand.
///
/// The interval boundary decides whether a warning is due, so supplying the
/// time is what makes that boundary a decision of this test rather than of the
/// scheduler.
#[derive(Default)]
struct ManualClock {
    /// Current test-controlled timestamp in milliseconds.
    now_ms: AtomicU64,
}

impl Clock for ManualClock {
    fn now_millis(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }
}

impl ManualClock {
    /// Move the clock past the warner's one-second interval.
    fn elapse_interval(&self) {
        self.now_ms.fetch_add(1000, Ordering::Relaxed);
    }
}

/// Build a record for a handler that will refuse it.
fn record(message: &str) -> FemtoLogRecord {
    FemtoLogRecord::new("test.logger", FemtoLevel::Info, message)
}

/// Render the warning naming `cause`, for comparison against captured messages.
fn drop_warning(count: u64, cause: &str) -> String {
    format!(
        concat!(
            "FemtoHTTPHandler dropped {count} records since the last report; ",
            "triggering cause: {cause}"
        ),
        count = count,
        cause = cause
    )
}

/// Count the captured messages equal to `expected`.
///
/// The test logger is process-global and the rest of this binary runs in
/// parallel, so the racing test below cannot assume its messages are the only
/// ones in the buffer. Counting exact matches keeps a neighbour's warning out of
/// the comparison while still failing on a second report from this test.
fn occurrences(messages: &[String], expected: &str) -> usize {
    messages
        .iter()
        .filter(|message| *message == expected)
        .count()
}

/// A due warning that carries two causes names only the one that triggered it.
///
/// Scenario: a queue-full drop warns and opens the interval, a serialization
/// failure is suppressed and stays pending, and the next interval's permanent
/// error reports both. Invariant: the message reports the cumulative count and
/// names only the cause whose refusal asked for the report, rather than
/// claiming the whole count happened for that cause.
///
/// The suppressed cause deliberately differs from the triggering cause. If it
/// did not, a warning that named a pending drop's cause instead of the
/// triggering one would produce the same message and pass unnoticed.
#[test]
#[serial_test::serial]
fn a_cumulative_warning_names_only_the_triggering_cause() {
    install_test_logger();
    let clock = Arc::new(ManualClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone());

    warn_drops_naming_cause(&warner, "queue full");
    warn_drops_naming_cause(&warner, "serialization failure");
    clock.elapse_interval();
    warn_drops_naming_cause(&warner, "permanent error");

    let cumulative: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message.contains("dropped 2 records"))
        .collect();
    assert_eq!(
        cumulative.len(),
        1,
        "the second interval should report both pending drops once"
    );
    assert!(
        cumulative[0]
            .message
            .contains("triggering cause: permanent error"),
        "the warning must name the cause that triggered it: {:?}",
        cumulative[0].message
    );
    assert!(
        !cumulative[0].message.contains("serialization failure"),
        "the warning must not attribute the suppressed drop's cause to the \
         triggering cause: {:?}",
        cumulative[0].message
    );
    assert!(
        !cumulative[0].message.contains("queue full"),
        "the warning must not name the cause that opened the earlier interval: \
         {:?}",
        cumulative[0].message
    );
}

/// Two threads reporting different causes race for one expired interval.
///
/// Scenario: one drop opens a real interval, two further drops from different
/// causes are recorded while it is open and therefore suppressed, the clock
/// then passes the interval, and both causes' reporters run at the same
/// instant. Invariant: exactly one report is emitted, it carries both pending
/// drops, and the cause it names belongs to the caller whose callback actually
/// ran — a shared "most recent cause" slot would let the losing caller relabel
/// a report it did not make.
///
/// The two reporters spell out the closure `warn_drops_naming_cause` passes to
/// `warn_if_due`, because the test must see which closure ran to identify the
/// winner. They record nothing themselves: both drops were already recorded
/// through the production helper inside the suppression window.
#[test]
#[serial_test::serial]
fn racing_causes_report_both_drops_and_name_the_trigger() {
    install_test_logger();
    let clock = Arc::new(ManualClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone());

    // The first report opens a real interval; the next two are suppressed and
    // stay pending, each recorded with the cause it will be reported under.
    // The `(race)` marker keeps these messages attributable while neighbouring
    // tests log through the same buffer.
    warn_drops_naming_cause(&warner, "queue full (race)");
    warn_drops_naming_cause(&warner, "queue full (race)");
    warn_drops_naming_cause(&warner, "worker disconnected (race)");
    let opening: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(&opening, &drop_warning(1, "queue full (race)")),
        1,
        "only the first report should escape the interval: {opening:?}"
    );
    clock.elapse_interval();

    let reports = Mutex::new(Vec::new());
    let start = Arc::new(Barrier::new(2));
    thread::scope(|scope| {
        for cause in ["queue full (race)", "worker disconnected (race)"] {
            let warner = &warner;
            let reports = &reports;
            let start = Arc::clone(&start);
            scope.spawn(move || {
                start.wait();
                warner.warn_if_due(|count| {
                    reports.lock().expect("reports lock").push((cause, count));
                    warn_cumulative_drops(count, cause);
                });
            });
        }
    });

    let reports = reports.into_inner().expect("reports lock");
    assert_eq!(
        reports.len(),
        1,
        "only the caller whose callback ran is the interval winner"
    );
    let (cause, count) = reports[0];
    let messages: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(&messages, &drop_warning(count, cause)),
        1,
        "the report must be the winner's own count and cause: {messages:?}"
    );
    let loser_cause = if cause == "queue full (race)" {
        "worker disconnected (race)"
    } else {
        "queue full (race)"
    };
    assert_eq!(
        occurrences(&messages, &drop_warning(count, loser_cause)),
        0,
        "the losing caller must not relabel the winner's report: {messages:?}"
    );
    assert_eq!(count, 2, "both suppressed drops must be in the report");
    assert_eq!(warner.pending_drops(), 0);
}

/// A flush reports the count the interval held back, without inventing a cause.
///
/// The first drop warns and opens the interval; the next two are suppressed and
/// stay pending. The flush reports that pending count of two with the
/// cause-neutral wording, rather than naming either suppressed drop's cause as
/// the reason for the whole count.
#[test]
#[serial_test::serial]
fn a_flush_reports_the_cumulative_count_without_a_cause() {
    install_test_logger();
    let clock = Arc::new(ManualClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(1), clock);

    warn_drops_naming_cause(&warner, "queue full");
    warn_drops_naming_cause(&warner, "serialization failure");
    warn_drops_naming_cause(&warner, "permanent error");
    flush_drops(&warner);

    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message == "FemtoHTTPHandler dropped 2 records since the last report")
        .collect();
    assert_eq!(
        reported.len(),
        1,
        "the flush should report both held-back drops with the cause-neutral \
         wording"
    );
}

/// Render the cause-neutral report for `count` pending drops.
fn neutral_drop_report(count: u64) -> String {
    format!("FemtoHTTPHandler dropped {count} records since the last report")
}

/// A closed handler holding one drop the warning interval suppressed.
///
/// The first refused record warns and opens the interval; the second is held
/// back, so whatever reports next must cover that one drop.
///
/// The interval is pinned far beyond the test's lifetime rather than left to
/// the wall clock. With the real interval, a second refusal that landed more
/// than one interval after the first would warn on its own, leaving the later
/// flush, close or destruction with nothing to report - a failure that says
/// nothing about the code under test.
fn closed_handler_holding_one_drop() -> FemtoHTTPHandler {
    let config = HTTPHandlerConfig {
        warn_interval: Duration::from_secs(3600),
        ..HTTPHandlerConfig::default()
    };
    let mut handler = FemtoHTTPHandler::with_config(config);
    handler.close();
    let _ = take_logged_messages();
    assert!(handler.handle(record("first refusal")).is_err());
    assert!(handler.handle(record("held back")).is_err());

    // Each refusal reports a count of one if it warns at all, so a second
    // occurrence would mean the interval had already elapsed and the second
    // drop had been consumed. A single opening warning therefore proves
    // exactly one drop is still pending for the later lifecycle call.
    let messages: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(&messages, &drop_warning(1, "handler closed")),
        1,
        "the first refusal should be the interval's only warning: {messages:?}"
    );
    handler
}

/// Assert exactly one cause-neutral report of the held-back drop was emitted.
fn assert_held_back_drop_reported(context: &str) {
    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message == neutral_drop_report(1))
        .collect();
    assert_eq!(reported.len(), 1, "{context}");
}

/// The handler's own flush reports drops refused after close.
///
/// A closed handler still records refused `handle()` calls. The first refused
/// record warns and opens the interval; the second is held back. `flush()` must
/// report that held-back count even though it returns `false` for a closed
/// handler, and a second flush must have nothing left to say.
#[test]
#[serial_test::serial]
fn flush_reports_drops_refused_after_close() {
    install_test_logger();
    let handler = closed_handler_holding_one_drop();

    assert!(!handler.flush(), "a closed handler's flush reports false");
    assert_held_back_drop_reported(
        "the flush should report the count the interval was holding back",
    );

    assert!(!handler.flush());
    // A detached worker from an earlier test can still log after this test
    // cleared the buffer, so only a second matching report proves the flush
    // had nothing to say; an empty global buffer would not.
    let messages: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(&messages, &neutral_drop_report(1)),
        0,
        "a second flush has nothing left to report: {messages:?}"
    );
}

/// A close that finds the worker already gone still reports pending drops.
#[test]
#[serial_test::serial]
fn close_reports_drops_refused_after_an_earlier_close() {
    install_test_logger();
    let mut handler = closed_handler_holding_one_drop();
    let _ = take_logged_messages();

    handler.close();

    assert_held_back_drop_reported("a close must report drops recorded after an earlier close");
}

/// Destroying a closed handler reports the drops it refused after close.
///
/// The drop path used to return as soon as the worker handle was gone, which
/// left a suppressed count on a warner nobody would ever read again.
#[test]
#[serial_test::serial]
fn dropping_a_closed_handler_reports_drops_refused_after_close() {
    install_test_logger();
    let handler = closed_handler_holding_one_drop();
    let _ = take_logged_messages();

    drop(handler);

    assert_held_back_drop_reported("destruction must not discard drops refused after close");
}
