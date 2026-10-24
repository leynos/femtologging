//! Rate-limited, recursion-resistant warnings for callback failures.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::rate_limited_warner::RateLimitedWarner;

/// Shared warning state so cloned filters do not emit duplicate failure notices.
#[derive(Clone)]
pub(super) struct CallbackFailureWarner(Arc<CallbackFailureWarningState>);

/// Coordinates rate limiting and re-entrancy protection for callback warnings.
struct CallbackFailureWarningState {
    /// Counts callback failures and decides when a warning is due.
    rate_limiter: RateLimitedWarner,
    /// Prevents a warning callback from recursively reporting another warning.
    warning_active: AtomicBool,
}

impl CallbackFailureWarner {
    /// Record a failure and emit its summary only when rate limiting allows it.
    pub(super) fn report(&self, mut warning: impl FnMut(u64)) {
        self.0.rate_limiter.record_drop();
        self.0.rate_limiter.warn_if_due(|failure_count| {
            let Some(_guard) = WarningActiveGuard::acquire(&self.0.warning_active) else {
                return;
            };
            warning(failure_count);
        });
    }
}

impl Default for CallbackFailureWarner {
    fn default() -> Self {
        Self(Arc::new(CallbackFailureWarningState {
            rate_limiter: RateLimitedWarner::default(),
            warning_active: AtomicBool::new(false),
        }))
    }
}

impl fmt::Debug for CallbackFailureWarner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CallbackFailureWarner")
    }
}

/// Resets the warning re-entrancy flag when the guarded callback returns.
struct WarningActiveGuard<'a>(&'a AtomicBool);

impl WarningActiveGuard<'_> {
    /// Acquire exclusive warning-callback activity, or return `None` if already active.
    fn acquire(active: &AtomicBool) -> Option<WarningActiveGuard<'_>> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| WarningActiveGuard(active))
    }
}

impl Drop for WarningActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
