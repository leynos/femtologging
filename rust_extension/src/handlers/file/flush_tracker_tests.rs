//! Tests for the file handler's periodic flush tracker.

use super::*;
use crate::handlers::file::test_support;
use rstest::*;
use serial_test::serial;
use std::io::{self, Write};

/// Writer double that counts flushes and can return a controlled I/O error.
#[derive(Default)]
struct DummyWriter {
    /// Number of flush calls observed by the test.
    flushed: usize,
    /// Makes `flush` fail after recording the call.
    fail: bool,
}

impl Write for DummyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed += 1;
        if self.fail {
            Err(io::Error::other("flush failed"))
        } else {
            Ok(())
        }
    }
}

/// Provide a writer that can optionally fail its next flush.
#[fixture]
fn writer(#[default(false)] fail: bool) -> DummyWriter {
    DummyWriter { flushed: 0, fail }
}

/// One row of the tracker's flush-decision table.
///
/// The fields travel together as a single described case, which names what
/// each position means and keeps the parameterised test within the
/// argument-count budget.
#[derive(Clone, Copy, Debug)]
struct FlushCase {
    /// Flush threshold in records; zero disables periodic flushing.
    interval: usize,
    /// Records counted as written since the last flush.
    writes: usize,
    /// Whether the writer's flush should fail.
    is_failing: bool,
    /// Flushes the case must observe.
    expected_flushes: usize,
    /// Whether the call must report an error.
    expects_error: bool,
}

/// Verify when the periodic tracker flushes and when it propagates errors.
#[rstest]
#[case::flushes_when_due(FlushCase {
    interval: 2,
    writes: 2,
    is_failing: false,
    expected_flushes: 1,
    expects_error: false,
})]
#[case::propagates_error(FlushCase {
    interval: 1,
    writes: 1,
    is_failing: true,
    expected_flushes: 1,
    expects_error: true,
})]
#[case::waits_for_threshold(FlushCase {
    interval: 3,
    writes: 1,
    is_failing: false,
    expected_flushes: 0,
    expects_error: false,
})]
#[case::disabled_by_zero_interval(FlushCase {
    interval: 0,
    writes: 5,
    is_failing: false,
    expected_flushes: 0,
    expects_error: false,
})]
#[case::no_writes_yet(FlushCase {
    interval: 2,
    writes: 0,
    is_failing: false,
    expected_flushes: 0,
    expects_error: false,
})]
fn flush_if_due_cases(#[case] case: FlushCase) {
    let mut writer = DummyWriter {
        flushed: 0,
        fail: case.is_failing,
    };
    let mut tracker = FlushTracker::new(case.interval);
    tracker.writes = case.writes;
    let result = tracker.flush_if_due(&mut writer);
    assert_eq!(
        writer.flushed, case.expected_flushes,
        "unexpected flush count"
    );
    assert_eq!(
        result.is_err(),
        case.expects_error,
        "unexpected error verdict"
    );
}

/// Confirm write-triggered flush failures are logged as warnings.
#[rstest]
#[serial]
fn record_write_logs_warning_on_error(#[with(true)] mut writer: DummyWriter) {
    test_support::install_test_logger();
    let mut tracker = FlushTracker::new(1);
    let result = tracker.record_write(&mut writer);
    assert!(result.is_err());
    assert_eq!(writer.flushed, 1);

    let logs = test_support::take_logged_messages();
    let log = logs.into_iter().next().expect("no log produced");
    assert_eq!(log.level, log::Level::Warn);
    assert!(log.message.contains("after write"));
    assert!(log.message.contains("flush failed"));
}
