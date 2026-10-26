//! Behavioural regression test for removing a handler after queued work.

use std::sync::Arc;

use _femtologging_rs::{FemtoHandlerTrait, FemtoLevel};
use rstest::rstest;

use super::{
    FemtoLogger, HandlerTuple, attach_native_handler, fixtures::handler_tuple,
    shared_buffer::std::read_output,
};

#[rstest]
fn handler_can_be_removed(#[from(handler_tuple)] (buffer, handler): HandlerTuple) {
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(handler);
    let logger = FemtoLogger::new("core".to_string()).expect("test logger should be constructible");
    attach_native_handler(&logger, Arc::clone(&handler));
    logger.log(FemtoLevel::Info, "one");
    // The record is queued on the shared shard, so flushing the handler alone
    // can run before the worker delivers "one". `flush_handlers` waits on the
    // shard's barrier first, which makes the read deterministic on a loaded
    // runner; a fixed sleep only makes it likely.
    assert!(
        logger.flush_handlers(),
        "the shard and handler should drain before the output is read"
    );
    let output = read_output(&buffer);
    assert!(output.contains("one"));
    assert!(logger.remove_handler(&handler));
    logger.log(FemtoLevel::Info, "two");
    drop(logger);
    handler.flush();
    drop(handler);
    let output = read_output(&buffer);
    assert!(!output.contains("two"));
}
