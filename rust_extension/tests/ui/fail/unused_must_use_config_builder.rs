//! Compile-fail UI test for discarded immutable config builders.
//!
//! The config and logger builders are immutable: every setter consumes the
//! builder and returns a new one, and the setter carries `#[must_use]`. A
//! caller that discards the result therefore loses the whole chain, so the
//! deny below turns that silence into a build error.
#![deny(unused_must_use)]

use _femtologging_rs::{ConfigBuilder, FemtoLevel, LoggerConfigBuilder};

fn main() {
    ConfigBuilder::new().with_version(1);
    LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
}