//! Compile-pass coverage for `HandleExpect` smart-pointer wrappers.

#[path = "../../../tests/test_utils/handle_expect.rs"]
mod handle_expect;

use std::sync::Arc;

use _femtologging_rs::FemtoLogRecord;
use handle_expect::HandleExpect;

struct AcceptingHandler;

impl HandleExpect for AcceptingHandler {
    fn expect_handle(&self, _record: FemtoLogRecord) {}
}

fn assert_handle_expect<T: HandleExpect>() {
}

fn main() {
    assert_handle_expect::<&AcceptingHandler>();
    assert_handle_expect::<Arc<AcceptingHandler>>();
    assert_handle_expect::<Box<AcceptingHandler>>();
}
