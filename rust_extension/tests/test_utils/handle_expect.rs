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

use _femtologging_rs::{
    FemtoFileHandler,
    FemtoHandlerTrait,
    FemtoLogRecord,
    FemtoStreamHandler,
    HandlerError,
};

#[track_caller]
fn assert_handle_succeeded(result: Result<(), HandlerError>, context: &str) {
    match result {
        Ok(()) => {}
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Submit a record to a handler, panicking with a descriptive message if the
/// handler rejects it.
pub trait HandleExpect {
    /// Submit `record`, panicking with receiver-specific context on rejection.
    ///
    /// # Panics
    ///
    /// Panics when the handler returns an error for `record`.
    fn expect_handle(&self, record: FemtoLogRecord);
}

impl HandleExpect for FemtoFileHandler {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) {
        assert_handle_succeeded(
            self.handle(record),
            "expected FemtoFileHandler to accept record",
        );
    }
}

impl HandleExpect for FemtoStreamHandler {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) {
        assert_handle_succeeded(
            self.handle(record),
            "expected FemtoStreamHandler to accept record",
        );
    }
}

impl HandleExpect for dyn FemtoHandlerTrait {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) {
        assert_handle_succeeded(
            self.handle(record),
            "expected FemtoHandlerTrait object to accept record",
        );
    }
}

impl HandleExpect for dyn FemtoHandlerTrait + Send + Sync {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) {
        assert_handle_succeeded(
            self.handle(record),
            "expected FemtoHandlerTrait object to accept record",
        );
    }
}

impl<T: HandleExpect + ?Sized> HandleExpect for &T {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) { (**self).expect_handle(record); }
}

impl<T: HandleExpect + ?Sized> HandleExpect for Arc<T> {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) { (**self).expect_handle(record); }
}

impl<T: HandleExpect + ?Sized> HandleExpect for Box<T> {
    #[track_caller]
    fn expect_handle(&self, record: FemtoLogRecord) { (**self).expect_handle(record); }
}
