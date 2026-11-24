//! Tests for the drop counts a destroyed logger reports before it goes quiet.
//!
//! Split from `logger_tests_drop.rs` because that file is at its size limit.
//! The tests there cover what a drop waits for; this one covers what a drop
//! must say about records the warning interval was still holding back.

use serial_test::serial;

use crate::rate_limited_warner::{Clock, DEFAULT_WARN_INTERVAL, RateLimitedWarner};

use super::*;

/// A clock that never advances, so warning attempts cannot cross an interval.
struct FixedClock;

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        0
    }
}

/// A drop reports a count the warning interval is holding back.
///
/// Destruction is the only path that reports the logger's own warners, because
/// it is the only one without a caller deadline to overrun. After the drop
/// nothing is left to warn on the logger's behalf, so a suppressed interval
/// would take its final count with it. The second drop below warns nothing, so
/// without that flush the count would never be reported.
#[test]
#[serial]
fn drop_reports_the_pending_drop_count() {
    use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};

    install_test_logger();
    let mut logger =
        FemtoLogger::new("drop-report".to_string()).expect("test logger should be constructible");
    // A fixed clock keeps the second attempt inside the interval. With the
    // real clock, a pause longer than the interval would let it report the drop
    // this test needs the drop itself to report.
    logger.drop_warner = RateLimitedWarner::with_clock(
        DEFAULT_WARN_INTERVAL,
        Arc::new(FixedClock) as Arc<dyn Clock>,
    );
    // The first drop claims the interval; the second is suppressed and stays
    // pending, which is the state the drop has to report. The callback is a
    // no-op because what the first warning says is another test's subject.
    logger.drop_warner.record_drop();
    logger.drop_warner.warn_if_due(|_| {});
    logger.drop_warner.record_drop();
    logger.drop_warner.warn_if_due(|_| {});
    let _ = take_logged_messages();
    assert_eq!(
        logger.drop_warner.pending_drops(),
        1,
        "the second drop should be held back by the interval",
    );

    drop(logger);

    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message == "FemtoLogger: dropped 1 records; reported by flush")
        .collect();
    assert_eq!(
        reported.len(),
        1,
        "a dropped logger must report the count the interval was still holding: \
         reported {reported:?}",
    );
}
