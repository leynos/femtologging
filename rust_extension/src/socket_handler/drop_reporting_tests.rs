//! Contracts for what the socket handler reports when it drops records.
//!
//! The enqueue path and the worker share one counter, so a warning can carry
//! drops from several causes and a count that spans more than one interval
//! after a suppression. These tests pin the two consequences: a cumulative
//! warning names only its own drop's cause as the latest, and a report that
//! arrives on flush, close or destruction still accounts for drops the
//! interval was holding back.
//!
//! Included with `#[path]` from `tests.rs`, like `close_tests.rs`, so the
//! fixture and helpers in that file stay in scope.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::handler::FemtoHandlerTrait;
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::rate_limited_warner::{Clock, RateLimitedWarner};
use crate::socket_handler::warning::{flush_drops, warn_drops_naming_latest};
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

/// A due warning that carries two causes names only the second as the latest.
///
/// Scenario: the first drop warns and opens the interval, a second drop from a
/// different cause is suppressed and stays pending, and the next interval
/// reports both. Invariant: the message reports the cumulative count and says
/// which cause was the most recent, rather than claiming the whole count
/// happened for the cause that happened to trigger the report.
#[test]
#[serial_test::serial]
fn a_cumulative_warning_names_only_the_latest_cause() {
    install_test_logger();
    let clock = Arc::new(ManualClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone());

    warn_drops_naming_latest(&warner, "connect failure: refused");
    warn_drops_naming_latest(&warner, "write error");
    clock.elapse_interval();
    warn_drops_naming_latest(&warner, "write error");

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
        cumulative[0].message.contains("latest cause: write error"),
        "the warning must name the most recent cause: {:?}",
        cumulative[0].message
    );
    assert!(
        !cumulative[0].message.contains("due to write errors"),
        "the warning must not attribute the earlier connect failure to the \
         latest cause: {:?}",
        cumulative[0].message
    );
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

    warn_drops_naming_latest(&warner, "queue full");
    warn_drops_naming_latest(&warner, "connect failure: refused");
    warn_drops_naming_latest(&warner, "write error");
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

/// Close a handler and discard the messages its shutdown produced.
fn closed_handler() -> FemtoSocketHandler {
    let mut handler = FemtoSocketHandler::with_config(SocketHandlerConfig::default());
    handler.close();
    let _ = take_logged_messages();
    handler
}

/// A closed handler holding one drop the warning interval suppressed.
///
/// The first refused record warns and opens the interval; the second is held
/// back, so whatever reports next must cover that one drop.
fn closed_handler_holding_one_drop() -> FemtoSocketHandler {
    let handler = closed_handler();
    assert!(handler.handle(record("first refusal")).is_err());
    assert!(handler.handle(record("held back")).is_err());
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
    let warned: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message.contains("latest cause: handler closed"))
        .collect();
    assert_eq!(
        warned.len(),
        1,
        "the first refusal should warn once inside the interval"
    );

    assert!(!handler.flush(), "a closed handler's flush reports false");
    assert_held_back_drop_reported(
        "the flush should report the count the interval was holding back",
    );

    assert!(!handler.flush());
    assert!(
        take_logged_messages().is_empty(),
        "a second flush has nothing left to report"
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
