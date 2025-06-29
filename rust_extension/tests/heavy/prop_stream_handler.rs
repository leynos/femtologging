use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler, FemtoLogRecord};
use itertools::iproduct;
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
    fn prop_stream_handler_writes(
        ref messages in proptest::collection::vec("[^\n]*", 1..5),
        ref logger_names in proptest::collection::vec("[a-zA-Z_][a-zA-Z0-9_]{0,10}", 1..3),
        ref log_levels in proptest::collection::vec(prop_oneof![
            Just("INFO"),
            Just("DEBUG"),
            Just("WARN"),
            Just("ERROR"),
            Just("TRACE"),
        ], 1..3)
    ) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = FemtoStreamHandler::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);

        let mut expected = String::new();
        for (logger, level, msg) in iproduct!(logger_names, log_levels, messages) {
            handler.handle(FemtoLogRecord::new(logger, level, msg));
            expected.push_str(&format!("{} [{}] {}\n", logger, level, msg));
        }
        drop(handler);

        let output = read_output(&buffer);
        prop_assert_eq!(output, expected);
    }
}
