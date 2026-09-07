//! Property-based tests for `FemtoStreamHandler`.
//!
//! These tests generate random logger names, levels, and messages to verify
//! that the handler correctly writes each record without losing data.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use _femtologging_rs::{DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler};
use itertools::iproduct;
use proptest::prelude::*;

use crate::handle_expect::HandleExpect;
use crate::shared_buffer::std::SharedBuf;
use crate::shared_buffer::std::read_output;

proptest! {
    #[test]
    // Each case spins up a handler worker thread, so the suite is far slower
    // than an ordinary property test; it runs in the nightly heavy job.
    #[ignore = "spawns a worker thread per generated case; too slow for the ordinary gate"]
    fn prop_stream_handler_writes(
        ref messages in proptest::collection::vec("[^\n]*", 1..5),
        ref logger_names in proptest::collection::vec("[a-zA-Z_][a-zA-Z0-9_]{0,10}", 1..3),
        ref log_levels in proptest::collection::vec(prop_oneof![
            Just(FemtoLevel::Info),
            Just(FemtoLevel::Debug),
            Just(FemtoLevel::Warn),
            Just(FemtoLevel::Error),
            Just(FemtoLevel::Trace),
        ], 1..3)
    ) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handler = FemtoStreamHandler::new(
            SharedBuf::new(Arc::clone(&buffer)),
            DefaultFormatter,
        );

        let mut expected = String::new();
        for (logger, level, msg) in iproduct!(logger_names, log_levels, messages) {
            handler.expect_handle(FemtoLogRecord::new(logger, *level, msg));
            writeln!(&mut expected, "{logger} [{}] {msg}", level.as_str())
                .map_err(|error|TestCaseError::fail(error.to_string()))?;
        }
        drop(handler);

        let output = read_output(&buffer).expect("buffer output should be valid UTF-8");
        prop_assert_eq!(output, expected);
    }
}
