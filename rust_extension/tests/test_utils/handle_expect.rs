//! Ergonomic `handle` wrapper shared by the handler integration tests.
//!
//! `FemtoHandlerTrait::handle` returns a `Result`, so every call site would
//! otherwise need its own `.expect(...)`. `HandleExpect` centralizes that
//! boilerplate while keeping the failure message specific to the receiver
//! type. Blanket implementations cover the smart-pointer wrappers the tests
//! use (`&T`, `Arc<T>`, `Box<T>`), so a handler behind any of them can be
//! driven directly.
//!
//! ```ignore
//! use test_utils::handle_expect::HandleExpect;
//!
//! handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "hi"));
//! ```

use std::sync::Arc;

use _femtologging_rs::{FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord, FemtoStreamHandler};

/// Submit a record to a handler, panicking with a descriptive message if the
/// handler rejects it.
pub trait HandleExpect {
    /// Submit `record`, panicking with receiver-specific context on rejection.
    fn expect_handle(&self, record: FemtoLogRecord);
}

impl HandleExpect for FemtoFileHandler {
    fn expect_handle(&self, record: FemtoLogRecord) {
        self.handle(record)
            .expect("expected FemtoFileHandler to accept record");
    }
}

impl HandleExpect for FemtoStreamHandler {
    fn expect_handle(&self, record: FemtoLogRecord) {
        self.handle(record)
            .expect("expected FemtoStreamHandler to accept record");
    }
}

impl HandleExpect for dyn FemtoHandlerTrait {
    fn expect_handle(&self, record: FemtoLogRecord) {
        self.handle(record)
            .expect("expected FemtoHandlerTrait object to accept record");
    }
}

impl HandleExpect for dyn FemtoHandlerTrait + Send + Sync {
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
