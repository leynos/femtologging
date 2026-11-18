//! Negative probe: default-feature consumers cannot call the test-only
//! constructor.
//!
//! This crate is expected to fail to compile. It names
//! `FemtoFileHandler::with_writer_for_test`, which exists only for unit tests
//! and opt-in `test-util` builds. The function item is bound rather than
//! called: a diverging argument would make the call unreachable and trip
//! `unreachable_code` under `-D warnings`, which CI enables.

use std::io::Cursor;

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler};

/// Name the test-only constructor so its absence fails name resolution.
pub fn test_only_constructor() {
    let _constructor =
        FemtoFileHandler::with_writer_for_test::<Cursor<Vec<u8>>, DefaultFormatter>;
}