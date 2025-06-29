use loom::sync::{Arc, Mutex};
use loom::thread;
use std::io::{self, Write};

use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler, FemtoLogRecord};

#[derive(Clone)]
struct LoomBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LoomBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut lock = self.0.lock().unwrap();
        lock.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
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
        t.join().unwrap();
        drop(handler);
        let mut lines: Vec<_> = read_output(&buffer).lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["core [INFO] msg", "core [INFO] msg2"]);
    });
}
