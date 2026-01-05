//! Property-based tests for `FemtoStreamHandler`.
//!
//! These tests generate random logger names, levels, and messages to verify
//! that the handler correctly writes each record without losing data.

use std::io::{self, Write};

use _femtologging_rs::{DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler};
use itertools::iproduct;
use proptest::prelude::*;

#[path = "../test_utils/mod.rs"]
mod test_utils;
use std::sync::{Arc, Mutex};
use test_utils::shared_buffer::std::read_output;
use test_utils::std::SharedBuf;

proptest! {
    #[test]
    #[ignore]
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
            handler.handle(FemtoLogRecord::new(logger, *level, msg));
            expected.push_str(&format!("{} [{}] {}\n", logger, level.as_str(), msg));
        }
        drop(handler);

        let output = read_output(&buffer);
        prop_assert_eq!(output, expected);
    }
}
