use _femtologging_rs::{FemtoHandler, FemtoHandlerTrait, FemtoLogRecord};
use std::sync::Mutex;

#[derive(Default)]
struct DummyHandler {
    flushed: Mutex<bool>,
}

impl FemtoHandlerTrait for DummyHandler {
    fn handle(&self, _record: FemtoLogRecord) {}

    fn flush(&self) -> bool {
        let mut flag = self.flushed.lock().unwrap();
        *flag = true;
        true
    }
}

#[test]
fn default_handler_flush_returns_true() {
    let handler = FemtoHandler::default();
    assert!(handler.flush());
}

#[test]
fn overridden_flush_called_via_trait() {
    let handler = DummyHandler::default();
    let trait_obj: &dyn FemtoHandlerTrait = &handler;
    assert!(trait_obj.flush());
    assert!(*handler.flushed.lock().unwrap());
}
