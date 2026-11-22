//! Compile-pass coverage for the rotating builders' `const` setters.
//!
//! Each `const fn` below applies one public setter to a builder passed in as an
//! argument, so the fixture stops compiling the moment a setter loses its
//! `const` qualifier. The chains cannot start from `new(..)` inside a `const`
//! item — both constructors allocate — so the builder arrives as a parameter.
//!
//! `FileRotationStrategy::new` is crate-private and cannot be named here; it is
//! pinned in-crate by `src/handlers/rotating/core.rs`.

use std::num::NonZeroU64;

use _femtologging_rs::{RotatingFileHandlerBuilder, TimedRotatingFileHandlerBuilder};

/// An interval the timed builder accepts, established in const context.
const INTERVAL: NonZeroU64 = match NonZeroU64::new(60) {
    Some(value) => value,
    None => panic!("60 is non-zero"),
};

const fn rotating_builder_chain(
    builder: RotatingFileHandlerBuilder,
) -> RotatingFileHandlerBuilder {
    builder.with_max_bytes(1024).with_backup_count(3)
}

const fn timed_rotating_builder_chain(
    builder: TimedRotatingFileHandlerBuilder,
) -> TimedRotatingFileHandlerBuilder {
    builder.with_interval(INTERVAL).with_backup_count(3)
}

fn main() {
    // The const fns are the assertion; naming them keeps them used.
    let _: fn(RotatingFileHandlerBuilder) -> RotatingFileHandlerBuilder = rotating_builder_chain;
    let _: fn(TimedRotatingFileHandlerBuilder) -> TimedRotatingFileHandlerBuilder =
        timed_rotating_builder_chain;
}