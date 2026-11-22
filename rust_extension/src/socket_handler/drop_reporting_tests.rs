//! Contracts for what the socket handler reports when it drops records.
//!
//! One warner counts drops from several causes, and a suppression can make a
//! count span more than one interval. These tests pin the two consequences: a
//! cumulative warning names only the cause that triggered it, and a report that
//! arrives on close or destruction still accounts for drops the interval was
//! holding back. An open handler's flush reports through its worker, which
//! reports its own suppressed count as well.
//!
//! Declared with `#[path]` from `tests.rs`, like `close_tests.rs`. The
//! worker-side half of an open handler's flush report lives in
//! `worker_flush_reporting_tests.rs` instead, so both files stay clear of the
//! size limit.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use rstest::{fixture, rstest};

use crate::handler::FemtoHandlerTrait;
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::rate_limited_warner::{Clock, RateLimitedWarner};
use crate::socket_handler::warning::{flush_drops, warn_cumulative_drops, warn_drops_naming_cause};
use crate::socket_handler::{FemtoSocketHandler, SocketHandlerConfig};

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
            "FemtoSocketHandler dropped {count} records since the last report; ",
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
/// Scenario: the first drop warns and opens the interval, a second drop from a
/// different cause is suppressed and stays pending, and the next interval
/// reports both. Invariant: the message reports the cumulative count and names
/// only the cause whose refusal asked for the report, rather than claiming the
/// whole count happened for that cause.
#[test]
#[serial_test::serial]
fn a_cumulative_warning_names_only_the_triggering_cause() {
    install_test_logger();
    let clock = Arc::new(ManualClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone());

    warn_drops_naming_cause(&warner, "connect failure: refused");
    warn_drops_naming_cause(&warner, "write error");
    clock.elapse_interval();
    warn_drops_naming_cause(&warner, "write error");

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
            .contains("triggering cause: write error"),
        "the warning must name the cause that triggered it: {:?}",
        cumulative[0].message
    );
    assert!(
        !cumulative[0].message.contains("connect failure: refused"),
        "the warning must not attribute the earlier connect failure to the \
         triggering cause: {:?}",
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
    warn_drops_naming_cause(&warner, "connect failure: refused (race)");
    warn_drops_naming_cause(&warner, "connect failure: refused (race)");
    warn_drops_naming_cause(&warner, "write error (race)");
    let opening: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(
            &opening,
            &drop_warning(1, "connect failure: refused (race)")
        ),
        1,
        "only the first report should escape the interval: {opening:?}"
    );
    clock.elapse_interval();

    let reports = Mutex::new(Vec::new());
    let start = Arc::new(Barrier::new(2));
    thread::scope(|scope| {
        for cause in ["connect failure: refused (race)", "write error (race)"] {
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
    let loser_cause = if cause == "connect failure: refused (race)" {
        "write error (race)"
    } else {
        "connect failure: refused (race)"
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
    warn_drops_naming_cause(&warner, "connect failure: refused");
    warn_drops_naming_cause(&warner, "write error");
    flush_drops(&warner);

    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| {
            entry.message == "FemtoSocketHandler dropped 2 records since the last report"
        })
        .collect();
    assert_eq!(
        reported.len(),
        1,
        "the flush should report both held-back drops with the cause-neutral \
         wording"
    );
}

/// A closed handler holding one drop the warning interval suppressed.
///
/// The first refused record warns and opens the interval; the second is held
/// back, so whatever reports next must cover that one drop. The clock never
/// advances, which is what makes the suppression a property of this fixture
/// rather than a race against the real interval: wall-clock time elapsing
/// between the two refusals would let the second one warn too.
#[fixture]
fn closed_handler_holding_one_drop() -> FemtoSocketHandler {
    install_test_logger();
    let clock = Arc::new(ManualClock::default());
    let mut handler = FemtoSocketHandler::with_clock(SocketHandlerConfig::default(), clock);
    handler.close();
    let _ = take_logged_messages();
    assert!(handler.handle(record("first refusal")).is_err());
    assert!(handler.handle(record("held back")).is_err());
    assert_eq!(
        handler.pending_drops_for_test(),
        1,
        "the second refusal must stay pending inside the interval"
    );
    handler
}

/// Assert exactly one cause-neutral report of the held-back drop was emitted.
fn assert_held_back_drop_reported(context: &str) {
    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| {
            entry.message == "FemtoSocketHandler dropped 1 records since the last report"
        })
        .collect();
    assert_eq!(reported.len(), 1, "{context}");
}

/// A closed handler's flush is deadline-bound and reports nothing.
///
/// A closed handler still records refused `handle()` calls. The first refusal
/// warns and opens the interval; the second is held back. That count must not
/// be written through the process-wide sink here: a blocking sink would hold
/// the caller past the deadline the flush was given. The count stays pending
/// for the next `close()` or destruction, which are outside that bound.
#[rstest]
#[serial_test::serial]
fn a_closed_handler_flush_leaves_the_pending_count_for_close(
    closed_handler_holding_one_drop: FemtoSocketHandler,
) {
    let mut handler = closed_handler_holding_one_drop;
    let warned: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message.contains("triggering cause: handler closed"))
        .collect();
    assert_eq!(
        warned.len(),
        1,
        "the first refusal should warn once inside the interval"
    );

    assert!(!handler.flush(), "a closed handler's flush reports false");
    assert_eq!(
        handler.pending_drops_for_test(),
        1,
        "the flush must leave the held-back count pending"
    );
    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| {
            entry.message == "FemtoSocketHandler dropped 1 records since the last report"
        })
        .collect();
    assert!(
        reported.is_empty(),
        "a deadline-bound flush must not report the count: {reported:?}"
    );

    handler.close();

    assert_held_back_drop_reported("close must report the count the flush left pending");
}

/// A close that finds the worker already gone still reports pending drops.
#[rstest]
#[serial_test::serial]
fn close_reports_drops_refused_after_an_earlier_close(
    closed_handler_holding_one_drop: FemtoSocketHandler,
) {
    let mut handler = closed_handler_holding_one_drop;
    let _ = take_logged_messages();

    handler.close();

    assert_held_back_drop_reported("a close must report drops recorded after an earlier close");
}

/// Destroying a closed handler reports the drops it refused after close.
///
/// The drop path used to return as soon as the worker handle was gone, which
/// left a suppressed count on a warner nobody would ever read again.
#[rstest]
#[serial_test::serial]
fn dropping_a_closed_handler_reports_drops_refused_after_close(
    closed_handler_holding_one_drop: FemtoSocketHandler,
) {
    let handler = closed_handler_holding_one_drop;
    let _ = take_logged_messages();

    drop(handler);

    assert_held_back_drop_reported("destruction must not discard drops refused after close");
}
