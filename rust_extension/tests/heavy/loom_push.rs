//! Concurrency tests using Loom to verify push delivery order.
//!
//! These tests model concurrent logging via the `FemtoStreamHandler` to ensure
//! there are no race conditions when multiple threads push records.

use loom::sync::{Arc, Mutex};
use loom::thread;

use _femtologging_rs::{DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler};

use crate::handle_expect::HandleExpect;
use crate::shared_buffer::loom::SharedBuf as LoomBuf;
use crate::shared_buffer::loom::read_output;

#[test]
fn loom_stream_push_delivery() {
    loom::model(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(FemtoStreamHandler::new(
            LoomBuf::new(Arc::clone(&buffer)),
            DefaultFormatter,
        ));
        let h = Arc::clone(&handler);
        let t = thread::spawn(move || {
            h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "msg"));
        });
        handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "msg2"));
        t.join().expect("Thread panicked");
        drop(handler);
        let output = read_output(&buffer);
        let mut lines: Vec<_> = output.lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["core [INFO] msg", "core [INFO] msg2"]);
    });
}
