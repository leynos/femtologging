//! Compile-pass coverage for the deadline-aware `FemtoHandlerTrait::flush_until`.

use std::any::Any;
use std::time::{Duration, Instant};

use _femtologging_rs::{FemtoHandlerTrait, FemtoLogRecord, FlushContext, HandlerError};

/// Legacy handler that relies on the trait's default `flush_until`.
struct LegacyHandler;

impl FemtoHandlerTrait for LegacyHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Handler that opts into the deadline-aware flush contract.
struct DeadlineAwareHandler;

impl FemtoHandlerTrait for DeadlineAwareHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, context: &FlushContext) -> bool {
        context
            .deadline()
            .is_some_and(|deadline| deadline.checked_duration_since(context.now()).is_some())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Flush through the trait object, as the logger's worker does.
fn flush_before_deadline(handler: &dyn FemtoHandlerTrait) -> bool {
    let now = Instant::now();
    handler.flush_until(&FlushContext::new(now, now + Duration::from_secs(1)))
}

fn main() {
    let legacy: &dyn FemtoHandlerTrait = &LegacyHandler;
    let deadline_aware: &dyn FemtoHandlerTrait = &DeadlineAwareHandler;
    assert!(!flush_before_deadline(legacy), "the default must fail closed");
    assert!(flush_before_deadline(deadline_aware));
}
