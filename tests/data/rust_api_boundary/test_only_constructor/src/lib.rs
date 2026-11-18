//! Negative probe: default-feature consumers cannot call the test-only
//! constructor.
//!
//! This crate is expected to fail to compile. It calls
//! `FemtoFileHandler::with_writer_for_test`, which exists only for unit tests
//! and opt-in `test-util` builds. A diverging argument keeps the call free of
//! any other test-only name, so the only possible diagnostic is the missing
//! associated function.

use std::io::Cursor;

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler};

/// Call the test-only constructor with an explicitly typed argument.
pub fn scripted_handler() -> FemtoFileHandler {
    FemtoFileHandler::with_writer_for_test::<Cursor<Vec<u8>>, DefaultFormatter>(unimplemented!())
}