//! Property-based tests for `FemtoStreamHandler`.
//!
//! These tests generate random logger names, levels, and messages to verify
//! that the handler correctly writes each record without losing data.

use std::sync::{Arc, Mutex};

use _femtologging_rs::{DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler};
use itertools::iproduct;
use proptest::prelude::*;

use crate::handle_expect::HandleExpect;
use crate::shared_buffer::std::SharedBuf;
use crate::shared_buffer::std::read_output;

/// Characters Python's `str.splitlines()` treats as line boundaries.
///
/// This mirrors the default formatter's escaped set. The formatter keeps every
/// other character verbatim, including unassigned and private-use characters,
/// so the test must not simply call `char::escape_debug` on every character:
/// that would also escape characters the formatter writes unchanged.
const LINE_SEPARATORS: [char; 10] = [
    '\u{000A}', // LINE FEED
    '\u{000B}', // LINE TABULATION (VERTICAL TAB)
    '\u{000C}', // FORM FEED
    '\u{000D}', // CARRIAGE RETURN
    '\u{001C}', // FILE SEPARATOR
    '\u{001D}', // GROUP SEPARATOR
    '\u{001E}', // RECORD SEPARATOR
    '\u{0085}', // NEXT LINE (NEL)
    '\u{2028}', // LINE SEPARATOR
    '\u{2029}', // PARAGRAPH SEPARATOR
];

/// Return `message` as the default formatter writes it.
///
/// The generated messages may contain a line separator, and the property is
/// about the handler writing each record unchanged rather than about the
/// formatter's escaping, so the expected text applies the formatter's escape
/// to those characters instead of restricting the generator.
fn formatted_message(message: &str) -> String {
    let mut escaped = String::with_capacity(message.len());
    for character in message.chars() {
        if LINE_SEPARATORS.contains(&character) {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

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
            expected.push_str(&format!(
                "{} [{}] {}\n",
                logger,
                level.as_str(),
                formatted_message(msg)
            ));
        }
        drop(handler);

        let output = read_output(&buffer);
        prop_assert_eq!(output, expected);
    }
}
