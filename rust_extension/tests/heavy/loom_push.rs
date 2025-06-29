//! Concurrency tests using Loom to verify push delivery order.
//!
//! These tests model concurrent logging via the `FemtoStreamHandler` to ensure
//! there are no race conditions when multiple threads push records.

use loom::sync::{Arc, Mutex};
use loom::thread;
use std::io::{self, Write};

use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler, FemtoLogRecord};

#[derive(Clone)]
struct LoomBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LoomBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self
            .0
            .lock()
            .expect("Failed to acquire lock for LoomBuf write")
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self
            .0
            .lock()
            .expect("Failed to acquire lock for LoomBuf flush")
            .flush()
    }
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    let data = buffer
        .lock()
        .expect("Failed to acquire lock for reading output")
        .clone();
    String::from_utf8(data).expect("Failed to convert buffer contents to UTF-8")
}

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
