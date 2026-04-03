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
            Err(io::Error::new(io::ErrorKind::Other, "flush failed"))
        } else {
            Ok(())
        }
    }
}

#[fixture]
/// Provide a writer that can optionally fail its next flush.
fn writer(#[default(false)] fail: bool) -> DummyWriter {
    DummyWriter { flushed: 0, fail }
}

#[rstest]
#[case(2, 2, false, 1, false)]
#[case(1, 1, true, 1, true)]
#[case(3, 1, false, 0, false)]
#[case(0, 5, false, 0, false)]
#[case(2, 0, false, 0, false)]
/// Verify when the periodic tracker flushes and when it propagates errors.
fn flush_if_due_cases(
    #[case] interval: usize,
    #[case] writes: usize,
    #[case] _fail: bool,
    #[case] expected_flushes: usize,
    #[case] expect_error: bool,
    #[with(_fail)] mut writer: DummyWriter,
) {
    let mut tracker = FlushTracker::new(interval);
    tracker.writes = writes;
    let result = tracker.flush_if_due(&mut writer);
    assert_eq!(writer.flushed, expected_flushes);
    assert_eq!(result.is_err(), expect_error);
}

#[rstest]
#[serial]
/// Confirm write-triggered flush failures are logged as warnings.
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
