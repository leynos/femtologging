use loom::sync::{Arc, Mutex};
use loom::thread;
use std::io::{self, Write};

use _femtologging_rs::{DefaultFormatter, FemtoLogger, FemtoStreamHandler};

#[derive(Clone)]
struct LoomBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LoomBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("lock").write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().expect("lock").flush()
    }
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    let data = buffer.lock().expect("lock").clone();
    String::from_utf8(data).expect("utf8")
}

#[test]
#[ignore]
fn loom_shared_handler_dispatch() {
    loom::model(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(FemtoStreamHandler::new(LoomBuf(Arc::clone(&buffer)), DefaultFormatter));
        let mut l1 = FemtoLogger::new("a".to_string());
        let mut l2 = FemtoLogger::new("b".to_string());
        l1.add_handler(Arc::clone(&handler));
        l2.add_handler(Arc::clone(&handler));
        let t = thread::spawn(move || {
            l1.log("INFO", "one");
        });
        l2.log("INFO", "two");
        t.join().unwrap();
        drop(handler);
        drop(l2);
        drop(l1);
        let mut lines: Vec<_> = read_output(&buffer).lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["a [INFO] one", "b [INFO] two"]);
    });
}
