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

use _femtologging_rs::{
    DefaultFormatter,
    FemtoHandlerTrait,
    FemtoLevel,
    FemtoLogger,
    FemtoStreamHandler,
};
use loom::{
    sync::{Arc as LoomArc, Mutex as LoomMutex},
    thread,
};

use crate::shared_buffer::loom::{SharedBuf as LoomBuf, read_output};

/// A loom-instrumented byte buffer shared with a stream handler.
type LoomBuffer = LoomArc<LoomMutex<Vec<u8>>>;

/// Return a fresh loom-instrumented output buffer.
fn fresh_buffer() -> LoomBuffer { LoomArc::new(LoomMutex::new(Vec::new())) }

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
fn sorted_lines(buffer: &LoomBuffer) -> Result<Vec<String>, std::string::FromUtf8Error> {
    let output = read_output(buffer)?;
    let mut lines: Vec<String> = output.lines().map(str::to_owned).collect();
    lines.sort();
    Ok(lines)
}

#[test]
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
        assert_eq!(
            sorted_lines(&buf1).expect("buffer output should be valid UTF-8"),
            ["core [INFO] one", "core [INFO] two"]
        );
        assert_eq!(
            sorted_lines(&buf2).expect("buffer output should be valid UTF-8"),
            ["core [INFO] one", "core [INFO] two"]
        );
    });
}

#[test]
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
        assert_eq!(
            sorted_lines(&buffer).expect("buffer output should be valid UTF-8"),
            ["a [INFO] one", "b [INFO] two"]
        );
    });
}

#[test]
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
            sorted_lines(&shared_buf).expect("buffer output should be valid UTF-8"),
            ["l1 [INFO] one", "l2 [INFO] two"]
        );
        assert_eq!(
            read_output(&buf1).expect("buffer output should be valid UTF-8"),
            "l1 [INFO] one\n"
        );
        assert_eq!(
            read_output(&buf2).expect("buffer output should be valid UTF-8"),
            "l2 [INFO] two\n"
        );
    });
}

#[test]
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

        assert_eq!(
            read_output(&buf1).expect("buffer output should be valid UTF-8"),
            "core [INFO] hi\n"
        );
        assert_eq!(
            read_output(&buf2).expect("buffer output should be valid UTF-8"),
            "core [INFO] hi\n"
        );
        assert_eq!(
            read_output(&buf3).expect("buffer output should be valid UTF-8"),
            "core [INFO] hi\n"
        );
    });
}
