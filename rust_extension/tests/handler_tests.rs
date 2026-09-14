//! Tests for the default `FemtoHandler` and for trait-object dispatch of
//! `FemtoHandlerTrait` methods onto user-supplied implementations.

use _femtologging_rs::{FemtoHandler, FemtoHandlerTrait, FemtoLogRecord, HandlerError};
use std::sync::{Mutex, PoisonError};

#[derive(Default)]
struct DummyHandler {
    flushed: Mutex<bool>,
}

impl FemtoHandlerTrait for DummyHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        // A test double must not mask a genuine failure by panicking on a
        // poisoned lock, so recover the inner value instead.
        let mut flag = self.flushed.lock().unwrap_or_else(PoisonError::into_inner);
        *flag = true;
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn default_handler_flush_returns_true() {
    let handler = FemtoHandler;
    assert!(handler.flush());
}

#[test]
fn overridden_flush_called_via_trait() {
    let handler = DummyHandler::default();
    let trait_obj: &dyn FemtoHandlerTrait = &handler;
    assert!(trait_obj.flush());
    assert!(*handler.flushed.lock().expect("flushed flag mutex poisoned"));
}
