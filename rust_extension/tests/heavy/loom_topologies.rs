//! Concurrency tests for various logger/handler topologies.
//!
//! These tests leverage `loom` to explore possible thread interleavings
//! and ensure log records are routed correctly without duplication.
//!
//! Handlers and loggers are held in `std::sync::Arc` because
//! `FemtoLogger::add_handler` takes a `std::sync::Arc<dyn FemtoHandlerTrait>`;
//! only the shared output buffers use loom's primitives, since those are the
//! locations whose interleavings the model actually explores.

use std::sync::Arc;

use loom::sync::{Arc as LoomArc, Mutex as LoomMutex};
use loom::thread;

use _femtologging_rs::{
    DefaultFormatter, FemtoHandlerTrait, FemtoLevel, FemtoLogger, FemtoStreamHandler,
};

use crate::test_utils::shared_buffer::loom::SharedBuf as LoomBuf;
use crate::test_utils::shared_buffer::loom::read_output;

/// A loom-instrumented byte buffer shared with a stream handler.
type LoomBuffer = LoomArc<LoomMutex<Vec<u8>>>;

/// Return a fresh loom-instrumented output buffer.
fn fresh_buffer() -> LoomBuffer {
    LoomArc::new(LoomMutex::new(Vec::new()))
}

/// Return a default-formatting stream handler writing into `buffer`.
fn handler_for(buffer: &LoomBuffer) -> Arc<FemtoStreamHandler> {
    Arc::new(FemtoStreamHandler::new(
        LoomBuf::new(LoomArc::clone(buffer)),
        DefaultFormatter,
    ))
}

/// Return the buffer contents split into sorted lines.
///
/// Ordering between concurrently logged records is not specified, so tests
/// compare sorted lines rather than raw output.
fn sorted_lines(buffer: &LoomBuffer) -> Vec<String> {
    let mut lines: Vec<String> = read_output(buffer).lines().map(str::to_owned).collect();
    lines.sort();
    lines
}

// Registered as a test only under `--cfg loom`: see the module documentation
// for why these models cannot run against the current handler implementation.
#[cfg_attr(loom, test)]
#[allow(dead_code)]
fn loom_single_logger_multi_handlers() {
    loom::model(|| {
        let buf1 = fresh_buffer();
        let buf2 = fresh_buffer();
        let h1 = handler_for(&buf1);
        let h2 = handler_for(&buf2);
        let logger = FemtoLogger::new("core".to_string());
        logger.add_handler(h1.clone() as Arc<dyn FemtoHandlerTrait>);
        logger.add_handler(h2.clone() as Arc<dyn FemtoHandlerTrait>);
        let logger = Arc::new(logger);
        let l = Arc::clone(&logger);
        let t = thread::spawn(move || {
            let _ = l.log(FemtoLevel::Info, "one");
        });
        let _ = logger.log(FemtoLevel::Info, "two");
        t.join().expect("Thread panicked");
        drop(logger);
        drop(h1);
        drop(h2);
        assert_eq!(sorted_lines(&buf1), ["core [INFO] one", "core [INFO] two"]);
        assert_eq!(sorted_lines(&buf2), ["core [INFO] one", "core [INFO] two"]);
    });
}

// Registered as a test only under `--cfg loom`: see the module documentation
// for why these models cannot run against the current handler implementation.
#[cfg_attr(loom, test)]
#[allow(dead_code)]
fn loom_shared_handler_multi_loggers() {
    loom::model(|| {
        let buffer = fresh_buffer();
        let handler = handler_for(&buffer);
        let l1 = FemtoLogger::new("a".to_string());
        let l2 = FemtoLogger::new("b".to_string());
        l1.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
        l2.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
        let l1 = Arc::new(l1);
        let l2 = Arc::new(l2);
        let t = thread::spawn({
            let l1 = Arc::clone(&l1);
            move || {
                let _ = l1.log(FemtoLevel::Info, "one");
            }
        });
        let _ = l2.log(FemtoLevel::Info, "two");
        t.join().expect("Thread panicked");
        drop(l1);
        drop(l2);
        drop(handler);
        assert_eq!(sorted_lines(&buffer), ["a [INFO] one", "b [INFO] two"]);
    });
}

// Registered as a test only under `--cfg loom`: see the module documentation
// for why these models cannot run against the current handler implementation.
#[cfg_attr(loom, test)]
#[allow(dead_code)]
fn loom_multiple_loggers_multiple_handlers() {
    loom::model(|| {
        let shared_buf = fresh_buffer();
        let buf1 = fresh_buffer();
        let buf2 = fresh_buffer();
        let shared_handler = handler_for(&shared_buf);
        let h1 = handler_for(&buf1);
        let h2 = handler_for(&buf2);
        let l1 = FemtoLogger::new("l1".to_string());
        l1.add_handler(shared_handler.clone() as Arc<dyn FemtoHandlerTrait>);
        l1.add_handler(h1.clone() as Arc<dyn FemtoHandlerTrait>);
        let l2 = FemtoLogger::new("l2".to_string());
        l2.add_handler(shared_handler.clone() as Arc<dyn FemtoHandlerTrait>);
        l2.add_handler(h2.clone() as Arc<dyn FemtoHandlerTrait>);
        let l1 = Arc::new(l1);
        let l2 = Arc::new(l2);
        let t = thread::spawn({
            let l1 = Arc::clone(&l1);
            move || {
                let _ = l1.log(FemtoLevel::Info, "one");
            }
        });
        let _ = l2.log(FemtoLevel::Info, "two");
        t.join().expect("Thread panicked");
        drop(l1);
        drop(l2);
        drop(shared_handler);
        drop(h1);
        drop(h2);
        assert_eq!(
            sorted_lines(&shared_buf),
            ["l1 [INFO] one", "l2 [INFO] two"]
        );
        assert_eq!(read_output(&buf1), "l1 [INFO] one\n");
        assert_eq!(read_output(&buf2), "l2 [INFO] two\n");
    });
}

// Registered as a test only under `--cfg loom`: see the module documentation
// for why these models cannot run against the current handler implementation.
#[cfg_attr(loom, test)]
#[allow(dead_code)]
fn loom_concurrent_handler_addition() {
    loom::model(|| {
        let buf1 = fresh_buffer();
        let buf2 = fresh_buffer();
        let buf3 = fresh_buffer();
        let h1 = handler_for(&buf1);
        let h2 = handler_for(&buf2);
        let h3 = handler_for(&buf3);
        let logger = Arc::new(FemtoLogger::new("core".to_string()));

        let adders: Vec<_> = [
            h1.clone() as Arc<dyn FemtoHandlerTrait>,
            h2.clone() as Arc<dyn FemtoHandlerTrait>,
            h3.clone() as Arc<dyn FemtoHandlerTrait>,
        ]
        .into_iter()
        .map(|h| {
            let l = Arc::clone(&logger);
            thread::spawn(move || {
                l.add_handler(h);
            })
        })
        .collect();
        for (index, adder) in adders.into_iter().enumerate() {
            if adder.join().is_err() {
                panic!("handler-adding thread {index} panicked");
            }
        }

        let _ = logger.log(FemtoLevel::Info, "hi");
        drop(logger);
        drop(h1);
        drop(h2);
        drop(h3);

        assert_eq!(read_output(&buf1), "core [INFO] hi\n");
        assert_eq!(read_output(&buf2), "core [INFO] hi\n");
        assert_eq!(read_output(&buf3), "core [INFO] hi\n");
    });
}
