//! Concurrency tests for various logger/handler topologies.
//!
//! These tests leverage `loom` to explore possible thread interleavings
//! and ensure log records are routed correctly without duplication.

mod test_utils;
use test_utils::shared_buffer::loom::read_output;
use test_utils::shared_buffer::loom::SharedBuf as LoomBuf;
use loom::sync::{Arc, Mutex};
use loom::thread;
use std::io::Write;

use _femtologging_rs::{
    DefaultFormatter, FemtoLogger, FemtoHandlerTrait, FemtoStreamHandler,
};


#[test]
#[ignore]
fn loom_single_logger_multi_handlers() {
    loom::model(|| {
        let buf1 = Arc::new(Mutex::new(Vec::new()));
        let buf2 = Arc::new(Mutex::new(Vec::new()));
        let h1 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf1)),
            DefaultFormatter,
        ));
        let h2 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf2)),
            DefaultFormatter,
        ));
        let logger = FemtoLogger::new("core".to_string());
        logger.add_handler(h1.clone() as Arc<dyn FemtoHandlerTrait>);
        logger.add_handler(h2.clone() as Arc<dyn FemtoHandlerTrait>);
        let logger = Arc::new(logger);
        let l = Arc::clone(&logger);
        let t = thread::spawn(move || {
            l.log("INFO", "one");
        });
        logger.log("INFO", "two");
        t.join().expect("Thread panicked");
        drop(logger);
        drop(h1);
        drop(h2);
        let mut lines1: Vec<_> = read_output(&buf1).lines().collect();
        let mut lines2: Vec<_> = read_output(&buf2).lines().collect();
        lines1.sort();
        lines2.sort();
        assert_eq!(lines1, ["core [INFO] one", "core [INFO] two"]);
        assert_eq!(lines2, ["core [INFO] one", "core [INFO] two"]);
    });
}

#[test]
#[ignore]
fn loom_shared_handler_multi_loggers() {
    loom::model(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buffer)),
            DefaultFormatter,
        ));
        let l1 = FemtoLogger::new("a".to_string());
        let l2 = FemtoLogger::new("b".to_string());
        l1.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
        l2.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
        let l1 = Arc::new(l1);
        let l2 = Arc::new(l2);
        let t = thread::spawn({
            let l1 = Arc::clone(&l1);
            move || {
                l1.log("INFO", "one");
            }
        });
        l2.log("INFO", "two");
        t.join().expect("Thread panicked");
        drop(l1);
        drop(l2);
        drop(handler);
        let mut lines: Vec<_> = read_output(&buffer).lines().collect();
        lines.sort();
        assert_eq!(lines, ["a [INFO] one", "b [INFO] two"]);
    });
}

#[test]
#[ignore]
fn loom_multiple_loggers_multiple_handlers() {
    loom::model(|| {
        let shared_buf = Arc::new(Mutex::new(Vec::new()));
        let buf1 = Arc::new(Mutex::new(Vec::new()));
        let buf2 = Arc::new(Mutex::new(Vec::new()));
        let shared_handler = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&shared_buf)),
            DefaultFormatter,
        ));
        let h1 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf1)),
            DefaultFormatter,
        ));
        let h2 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf2)),
            DefaultFormatter,
        ));
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
                l1.log("INFO", "one");
            }
        });
        l2.log("INFO", "two");
        t.join().expect("Thread panicked");
        drop(l1);
        drop(l2);
        drop(shared_handler);
        drop(h1);
        drop(h2);
        let mut shared_lines: Vec<_> = read_output(&shared_buf).lines().collect();
        shared_lines.sort();
        assert_eq!(shared_lines, ["l1 [INFO] one", "l2 [INFO] two"]);
        assert_eq!(read_output(&buf1), "l1 [INFO] one\n");
        assert_eq!(read_output(&buf2), "l2 [INFO] two\n");
    });
}

#[test]
#[ignore]
fn loom_concurrent_handler_addition() {
    loom::model(|| {
        let buf1 = Arc::new(Mutex::new(Vec::new()));
        let buf2 = Arc::new(Mutex::new(Vec::new()));
        let buf3 = Arc::new(Mutex::new(Vec::new()));
        let h1 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf1)),
            DefaultFormatter,
        ));
        let h2 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf2)),
            DefaultFormatter,
        ));
        let h3 = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buf3)),
            DefaultFormatter,
        ));
        let logger = Arc::new(FemtoLogger::new("core".to_string()));

        let t1 = {
            let l = Arc::clone(&logger);
            let h = h1.clone() as Arc<dyn FemtoHandlerTrait>;
            thread::spawn(move || {
                l.add_handler(h);
            })
        };
        let t2 = {
            let l = Arc::clone(&logger);
            let h = h2.clone() as Arc<dyn FemtoHandlerTrait>;
            thread::spawn(move || {
                l.add_handler(h);
            })
        };
        let t3 = {
            let l = Arc::clone(&logger);
            let h = h3.clone() as Arc<dyn FemtoHandlerTrait>;
            thread::spawn(move || {
                l.add_handler(h);
            })
        };
        t1.join().expect("t1");
        t2.join().expect("t2");
        t3.join().expect("t3");

        logger.log("INFO", "hi");
        drop(logger);
        drop(h1);
        drop(h2);
        drop(h3);

        assert_eq!(read_output(&buf1), "core [INFO] hi\n");
        assert_eq!(read_output(&buf2), "core [INFO] hi\n");
        assert_eq!(read_output(&buf3), "core [INFO] hi\n");
    });
}

