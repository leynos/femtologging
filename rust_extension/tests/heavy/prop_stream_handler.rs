use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler, FemtoLogRecord};
use proptest::prelude::*;

#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
}

proptest! {
    #[test]
    #[ignore]
    fn prop_stream_handler_writes(ref messages in proptest::collection::vec(".*", 1..20)) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = FemtoStreamHandler::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);

        for msg in messages {
            handler.handle(FemtoLogRecord::new("core", "INFO", msg));
        }
        drop(handler);

        let output = read_output(&buffer);
        let expected = messages
            .iter()
            .map(|m| format!("core [INFO] {}\n", m))
            .collect::<String>();
        prop_assert_eq!(output, expected);
    }
}
