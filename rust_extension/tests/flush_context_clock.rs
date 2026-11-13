//! External tests for the clock seam [`FlushContext::with_clock`] accepts.
//!
//! The barrier hands every handler the same context, so a handler that samples
//! time through `FlushContext::now` observes the deadline transition the
//! logger formed. These tests drive that transition from outside the crate: a
//! custom handler implements [`MonotonicClock`] itself, advances the clock
//! while it runs, and the same context becomes expired without any real
//! waiting. That is the boundary a user-defined handler needs in order to test
//! its own flush contract deterministically.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use _femtologging_rs::{
    FemtoHandlerTrait, FemtoLogRecord, FlushContext, HandlerError, MonotonicClock, SharedClock,
};

/// A clock that only moves when the test moves it.
#[derive(Debug)]
struct SteppingClock {
    /// The instant all reported times are offset from.
    base: Instant,
    /// How far the clock has moved past `base`, in nanoseconds.
    offset_nanos: AtomicU64,
}

impl SteppingClock {
    /// Create a clock stopped at the current instant.
    fn new() -> Self {
        Self {
            base: Instant::now(),
            offset_nanos: AtomicU64::new(0),
        }
    }

    /// Move the clock forward by `delta`.
    fn advance(&self, delta: Duration) {
        let delta_nanos = u64::try_from(delta.as_nanos()).unwrap_or(u64::MAX);
        self.offset_nanos.fetch_add(delta_nanos, Ordering::Relaxed);
    }
}

impl MonotonicClock for SteppingClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_nanos(self.offset_nanos.load(Ordering::Relaxed))
    }
}

/// A handler that spends a slice of the barrier's budget and reports what it
/// observed.
struct BudgetSpendingHandler {
    /// The clock the barrier's context samples, shared with the test.
    clock: Arc<SteppingClock>,
    /// How much of the budget the handler spends before it finishes.
    spend: Duration,
    /// Whether the handler ever saw a live deadline on entry.
    saw_live: AtomicBool,
    /// Whether the handler ever saw an expired deadline on entry.
    saw_expired: AtomicBool,
}

impl BudgetSpendingHandler {
    /// Build a handler that advances `clock` by `spend`.
    fn new(clock: Arc<SteppingClock>, spend: Duration) -> Self {
        Self {
            clock,
            spend,
            saw_live: AtomicBool::new(false),
            saw_expired: AtomicBool::new(false),
        }
    }

    /// Report whether the handler was called with budget still remaining.
    fn saw_live(&self) -> bool {
        self.saw_live.load(Ordering::Relaxed)
    }

    /// Report whether the handler was called with the deadline already spent.
    fn saw_expired(&self) -> bool {
        self.saw_expired.load(Ordering::Relaxed)
    }
}

impl FemtoHandlerTrait for BudgetSpendingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, context: &FlushContext) -> bool {
        if context.is_expired() {
            self.saw_expired.store(true, Ordering::Relaxed);
            return false;
        }
        self.saw_live.store(true, Ordering::Relaxed);
        // Do the handler's own work, as a slow writer would. The injected
        // clock moves by exactly that much, so the deadline transition is
        // deterministic rather than a race against real time.
        self.clock.advance(self.spend);
        // The flush contract forbids reporting success after the deadline, so
        // a handler that finishes late must fail closed.
        !context.is_expired()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An injected clock makes the live-to-expired transition deterministic.
#[test]
fn an_injected_clock_drives_a_custom_handler_across_its_deadline() {
    let clock = Arc::new(SteppingClock::new());
    let started_at = clock.base;
    let deadline = started_at + Duration::from_secs(1);
    let injected: SharedClock = clock.clone();
    let context = FlushContext::with_clock(started_at, Some(deadline), injected);

    assert_eq!(context.deadline(), Some(deadline));
    assert!(
        !context.is_expired(),
        "a fresh context has its whole budget"
    );

    // A handler that finishes inside the budget confirms, without spending it.
    let prompt = BudgetSpendingHandler::new(Arc::clone(&clock), Duration::from_millis(400));
    assert!(
        prompt.flush_until(&context),
        "a handler that finishes inside its budget confirms the flush"
    );
    assert!(
        prompt.saw_live(),
        "the handler must observe the live deadline"
    );
    assert!(
        !context.is_expired(),
        "a prompt handler leaves budget in the context"
    );

    // The same context, spent past its deadline, is expired. The contract
    // forbids reporting success after the deadline, so the late handler must
    // fail closed.
    let late = BudgetSpendingHandler::new(Arc::clone(&clock), Duration::from_secs(1));
    assert!(
        !late.flush_until(&context),
        "a handler that finishes after its deadline must not report success"
    );
    assert!(
        late.saw_live(),
        "the late handler still starts inside the budget"
    );
    assert!(
        context.is_expired(),
        "the late handler spent the budget, so the same context is now expired"
    );
    assert_eq!(
        context.now(),
        started_at + Duration::from_millis(1400),
        "the context must sample the injected clock, not the real one"
    );

    // Once the deadline has passed, a further flush fails closed on entry.
    assert!(
        !late.flush_until(&context),
        "a handler that starts after the deadline must fail closed"
    );
    assert!(
        late.saw_expired(),
        "the third call must observe the expired deadline"
    );
}

/// An absent deadline fails closed rather than waiting unbounded.
#[test]
fn an_absent_deadline_reports_an_expired_context() {
    let clock = Arc::new(SteppingClock::new());
    let started_at = clock.base;
    let injected: SharedClock = clock.clone();
    let context = FlushContext::with_clock(started_at, None, injected);

    assert_eq!(context.deadline(), None);
    assert_eq!(context.remaining(), Duration::ZERO);
    assert!(context.is_expired());
    assert!(context.is_late(), "an absent deadline counts as passed");
}
