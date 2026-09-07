//! Tests for the file handler's periodic flush tracker.

use super::*;
use crate::handlers::file::test_support;
use rstest::*;
use serial_test::serial;
use std::io::{self, Write};

#[derive(Default)]
struct DummyWriter {
    flushed: usize,
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

#[derive(Debug)]
struct FlushIfDueCase {
    interval: usize,
    writes: usize,
    fail: bool,
    expected_flushes: usize,
    expect_error: bool,
}

/// Verify when the periodic tracker flushes and when it propagates errors.
#[rstest]
#[case(FlushIfDueCase { interval: 2, writes: 2, fail: false, expected_flushes: 1, expect_error: false })]
#[case(FlushIfDueCase { interval: 1, writes: 1, fail: true, expected_flushes: 1, expect_error: true })]
#[case(FlushIfDueCase { interval: 3, writes: 1, fail: false, expected_flushes: 0, expect_error: false })]
#[case(FlushIfDueCase { interval: 0, writes: 5, fail: false, expected_flushes: 0, expect_error: false })]
#[case(FlushIfDueCase { interval: 2, writes: 0, fail: false, expected_flushes: 0, expect_error: false })]
fn flush_if_due_cases(#[case] case: FlushIfDueCase) {
    let mut writer = DummyWriter {
        flushed: 0,
        fail: case.fail,
    };
    let mut tracker = FlushTracker::new(case.interval);
    tracker.writes = case.writes;
    let result = tracker.flush_if_due(&mut writer);
    assert_eq!(writer.flushed, case.expected_flushes);
    assert_eq!(result.is_err(), case.expect_error);
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
