//! Compile-fail coverage for the `#[must_use]` overflow-policy setter.
//!
//! The setter consumes and returns the builder, so the returned value is the
//! only copy carrying the requested policy. Discarding it is almost always a
//! mistake, which is why the setter is `#[must_use]`.

#![deny(unused_must_use)]

use _femtologging_rs::{FileHandlerBuilder, OverflowPolicy};

fn main() {
    FileHandlerBuilder::new("unused_policy.log").with_overflow_policy(OverflowPolicy::Block);
}