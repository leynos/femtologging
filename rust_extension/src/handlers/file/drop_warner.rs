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

/// Tracks dropped records and emits rate-limited warnings.
pub(crate) struct DropWarner {
    queue_full: RateLimitedWarner,
    closed: RateLimitedWarner,
    timeout: RateLimitedWarner,
    timeout_duration: Option<Duration>,
}

impl DropWarner {
    pub(crate) fn new(policy: OverflowPolicy) -> Self {
        Self {
            queue_full: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
            closed: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
            timeout: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
            timeout_duration: match policy {
                OverflowPolicy::Timeout(dur) => Some(dur),
                _ => None,
            },
        }
    }

    pub(crate) fn record(&self, reason: DropReason) {
        match reason {
            DropReason::QueueFull => {
                self.queue_full.record_drop();
                self.queue_full.warn_if_due(|count| {
                    warn!(
                        "FemtoFileHandler: {count} log records dropped because the queue was full"
                    );
                });
            }
            DropReason::Closed => {
                self.closed.record_drop();
                self.closed.warn_if_due(|count| {
                    warn!(
                        "FemtoFileHandler: {count} log records dropped after the handler was closed"
                    );
                });
            }
            DropReason::Timeout => {
                if let Some(timeout) = self.timeout_duration {
                    self.timeout.record_drop();
                    self.timeout.warn_if_due(|count| {
                        warn!(
                            "FemtoFileHandler: {count} log records dropped after timing out waiting for the worker thread (timeout: {timeout:?})"
                        );
                    });
                } else {
                    // Fall back to closed warnings if timeouts are disabled but callers
                    // still report them for consistency.
                    self.record(DropReason::Closed);
                }
            }
        }
    }

    pub(crate) fn flush(&self) {
        self.queue_full.flush(|count| {
            warn!("FemtoFileHandler: {count} log records dropped because the queue was full");
        });
        self.closed.flush(|count| {
            warn!("FemtoFileHandler: {count} log records dropped after the handler was closed");
        });
        if let Some(timeout) = self.timeout_duration {
            self.timeout.flush(|count| {
                warn!(
                    "FemtoFileHandler: {count} log records dropped after timing out waiting for the worker thread (timeout: {timeout:?})"
                );
            });
        }
    }
}
