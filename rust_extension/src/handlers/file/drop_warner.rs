//! Rate-limited warning helper for `FemtoFileHandler` drops.
//!
//! The file handler can reject records when its queue is full, the handler has
//! been closed, or when a timeout elapses. This helper mirrors the behaviour of
//! `FemtoStreamHandler` by coalescing repeated warnings and emitting periodic
//! summaries instead of logging on every failure.

use std::time::Duration;

use crate::rate_limited_warner::{RateLimitedWarner, DEFAULT_WARN_INTERVAL};
use log::warn;

use super::OverflowPolicy;

/// Categorises why a record was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropReason {
    QueueFull,
    Closed,
    Timeout,
}

const DROP_REASONS: [DropReason; 3] = [
    DropReason::QueueFull,
    DropReason::Closed,
    DropReason::Timeout,
];

impl DropReason {
    fn as_usize(self) -> usize {
        match self {
            DropReason::QueueFull => 0,
            DropReason::Closed => 1,
            DropReason::Timeout => 2,
        }
    }

    fn message(self, count: u64, timeout: Option<Duration>) -> String {
        match self {
            DropReason::QueueFull => format!(
                "FemtoFileHandler: {count} log records dropped because the queue was full"
            ),
            DropReason::Closed => format!(
                "FemtoFileHandler: {count} log records dropped after the handler was closed"
            ),
            DropReason::Timeout => format!(
                "FemtoFileHandler: {count} log records dropped after timing out waiting for the worker thread (timeout: {timeout:?})"
            ),
        }
    }

    fn requires_timeout(self) -> bool {
        matches!(self, DropReason::Timeout)
    }
}

/// Tracks dropped records and emits rate-limited warnings.
pub(crate) struct DropWarner {
    warners: [RateLimitedWarner; DROP_REASONS.len()],
    timeout_duration: Option<Duration>,
}

impl DropWarner {
    pub(crate) fn new(policy: OverflowPolicy) -> Self {
        let timeout_duration = match policy {
            OverflowPolicy::Timeout(dur) => Some(dur),
            _ => None,
        };
        Self {
            warners: std::array::from_fn(|_| RateLimitedWarner::new(DEFAULT_WARN_INTERVAL)),
            timeout_duration,
        }
    }

    pub(crate) fn record(&self, reason: DropReason) {
        let reason = match (reason, self.timeout_duration) {
            (DropReason::Timeout, None) => DropReason::Closed,
            (other, _) => other,
        };
        let warner = &self.warners[reason.as_usize()];
        warner.record_drop();
        let timeout = self.timeout_duration;
        warner.warn_if_due(|count| {
            warn!("{}", reason.message(count, timeout));
        });
    }

    pub(crate) fn flush(&self) {
        for reason in DROP_REASONS {
            if reason.requires_timeout() && self.timeout_duration.is_none() {
                continue;
            }
            let warner = &self.warners[reason.as_usize()];
            let timeout = self.timeout_duration;
            warner.flush(|count| {
                warn!("{}", reason.message(count, timeout));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
    use serial_test::serial;

    fn assert_single_warning(expected: &str) {
        let logs = take_logged_messages();
        assert_eq!(logs.len(), 1, "expected exactly one warning, got {logs:?}");
        assert_eq!(logs[0].message, expected);
    }

    #[test]
    #[serial]
    fn records_queue_full_warning() {
        install_test_logger();
        let warner = DropWarner::new(OverflowPolicy::Drop);

        warner.record(DropReason::QueueFull);

        assert_single_warning("FemtoFileHandler: 1 log records dropped because the queue was full");
    }

    #[test]
    #[serial]
    fn treats_timeout_as_closed_without_configured_duration() {
        install_test_logger();
        let warner = DropWarner::new(OverflowPolicy::Drop);

        warner.record(DropReason::Timeout);

        assert_single_warning(
            "FemtoFileHandler: 1 log records dropped after the handler was closed",
        );
    }

    #[test]
    #[serial]
    fn records_timeout_warning_when_duration_available() {
        install_test_logger();
        let warner = DropWarner::new(OverflowPolicy::Timeout(Duration::from_millis(250)));

        warner.record(DropReason::Timeout);

        assert_single_warning(
            "FemtoFileHandler: 1 log records dropped after timing out waiting for the worker thread (timeout: Some(250ms))",
        );
    }
}
