//! Rate-limited, recursion-resistant warnings for callback failures.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::rate_limited_warner::RateLimitedWarner;

#[derive(Clone)]
pub(super) struct CallbackFailureWarner(Arc<CallbackFailureWarningState>);

struct CallbackFailureWarningState {
    rate_limiter: RateLimitedWarner,
    warning_active: AtomicBool,
}

impl CallbackFailureWarner {
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

struct WarningActiveGuard<'a>(&'a AtomicBool);

impl WarningActiveGuard<'_> {
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
