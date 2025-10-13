//! Helper trait for asserting handler acceptance in tests.
//!
//! The trait wraps `handle` calls with an expectation that they succeed. Tests
//! use it to ensure records are dispatched without repeating `.expect(...)`
//! boilerplate across handler types and smart pointers.

use std::sync::Arc;

use _femtologging_rs::{FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord, FemtoStreamHandler};

/// Extension trait to assert that a handler accepts a record.
pub trait HandleExpect {
    /// Dispatch `record` and panic if the handler rejects it.
    fn expect_handle(&self, record: FemtoLogRecord);
}

impl HandleExpect for FemtoStreamHandler {
    fn expect_handle(&self, record: FemtoLogRecord) {
        self.handle(record)
            .expect("expected FemtoStreamHandler to accept record");
    }
}

impl HandleExpect for FemtoFileHandler {
    fn expect_handle(&self, record: FemtoLogRecord) {
        self.handle(record)
            .expect("expected FemtoFileHandler to accept record");
    }
}

impl HandleExpect for dyn FemtoHandlerTrait {
    fn expect_handle(&self, record: FemtoLogRecord) {
        self.handle(record)
            .expect("expected FemtoHandlerTrait object to accept record");
    }
}

impl<T: HandleExpect + ?Sized> HandleExpect for &T {
    fn expect_handle(&self, record: FemtoLogRecord) {
        (**self).expect_handle(record);
    }
}

impl<T: HandleExpect + ?Sized> HandleExpect for Arc<T> {
    fn expect_handle(&self, record: FemtoLogRecord) {
        (**self).expect_handle(record);
    }
}

impl<T: HandleExpect + ?Sized> HandleExpect for Box<T> {
    fn expect_handle(&self, record: FemtoLogRecord) {
        (**self).expect_handle(record);
    }
}
