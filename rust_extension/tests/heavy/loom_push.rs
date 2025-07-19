//! Concurrency tests using Loom to verify push delivery order.
//!
//! These tests model concurrent logging via the `FemtoStreamHandler` to ensure
//! there are no race conditions when multiple threads push records.

use loom::sync::{Arc as LoomArc, Mutex as LoomMutex};
use loom::thread;
use std::io::{self, Write};

use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler, FemtoLogRecord};

type Arc<T> = LoomArc<T>;
type Mutex<T> = LoomMutex<T>;

#[path = "../test_utils/shared_buffer.rs"]
mod shared_buffer;
use shared_buffer::{read_output, SharedBuf as LoomBuf};

#[test]
#[ignore]
fn loom_stream_push_delivery() {
    loom::model(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(FemtoStreamHandler::new(
            LoomBuf(Arc::clone(&buffer)),
            DefaultFormatter,
        ));
        let h = Arc::clone(&handler);
        let t = thread::spawn(move || {
            h.handle(FemtoLogRecord::new("core", "INFO", "msg"));
        });
        handler.handle(FemtoLogRecord::new("core", "INFO", "msg2"));
        t.join().expect("Thread panicked");
        drop(handler);
        let mut lines: Vec<_> = read_output(&buffer).lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["core [INFO] msg", "core [INFO] msg2"]);
    });
}
