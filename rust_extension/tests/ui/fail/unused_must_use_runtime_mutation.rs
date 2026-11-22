//! Compile-fail coverage for the `#[must_use]` runtime mutation builder.
//!
//! The transaction builder is immutable: every `with_*` method consumes the
//! builder and returns a new one, and the method carries `#[must_use]`. A
//! caller that discards the result applies nothing, so the deny below turns
//! that silence into a build error.

#![deny(unused_must_use)]

use _femtologging_rs::{RuntimeConfigBuilder, StreamHandlerBuilder};

fn main() {
    RuntimeConfigBuilder::new().with_handler("stderr", StreamHandlerBuilder::stderr());
}
