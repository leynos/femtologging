//! Negative probe: default-feature consumers cannot name test-only seams.
//!
//! This crate is expected to fail to compile. It names `TestConfig` and calls
//! `FemtoFileHandler::with_writer_for_test`, both of which exist only for unit
//! tests and opt-in `test-util` builds. If either symbol resolves under the
//! default features, a test-only seam has leaked into the production API.

use std::io::Cursor;

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler, TestConfig};

/// Build a handler over an in-memory writer through the test-only seam.
pub fn scripted_handler() -> FemtoFileHandler {
    let config = TestConfig::new(Cursor::new(Vec::new()), DefaultFormatter);
    FemtoFileHandler::with_writer_for_test(config)
}