//! Compile-pass coverage for `HandleExpect` smart-pointer wrappers.

#[path = "../../../tests/test_utils/handle_expect.rs"]
mod handle_expect;

use std::sync::Arc;

use _femtologging_rs::{FemtoLevel, FemtoLogRecord};
use handle_expect::HandleExpect;

struct AcceptingHandler;

impl HandleExpect for AcceptingHandler {
    fn expect_handle(&self, _record: FemtoLogRecord) {}
}

fn record() -> FemtoLogRecord {
    FemtoLogRecord::new("compile", FemtoLevel::Info, "record")
}

fn main() {
    let handler = AcceptingHandler;
    (&handler).expect_handle(record());
    Arc::new(AcceptingHandler).expect_handle(record());
    Box::new(AcceptingHandler).expect_handle(record());
}
