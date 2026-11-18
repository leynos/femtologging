//! Negative probe: default-feature consumers cannot name the test-only
//! handler configuration.
//!
//! This crate is expected to fail to compile. It imports `TestConfig`, which
//! exists only for unit tests and opt-in `test-util` builds. The companion
//! `test_only_constructor` probe covers the constructor separately so that one
//! resolution error cannot mask the other seam.

use std::io::Cursor;

use _femtologging_rs::{DefaultFormatter, TestConfig};

/// Build the test-only handler configuration over an in-memory writer.
pub fn scripted_config() -> TestConfig<Cursor<Vec<u8>>, DefaultFormatter> {
    TestConfig::new(Cursor::new(Vec::new()), DefaultFormatter)
}