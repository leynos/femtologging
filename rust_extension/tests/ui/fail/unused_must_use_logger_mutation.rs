//! Compile-fail coverage for the `#[must_use]` logger mutation builder.
//!
//! Every mutation setter consumes the builder and returns a new one, and the
//! setter carries `#[must_use]`. A caller that discards the result loses the
//! whole mutation, so the deny below turns that silence into a build error.

#![deny(unused_must_use)]

use _femtologging_rs::{FemtoLevel, LoggerMutationBuilder};

fn main() {
    LoggerMutationBuilder::new().with_level(FemtoLevel::Info);
    LoggerMutationBuilder::new().replace_handlers(["stderr"]);
}
