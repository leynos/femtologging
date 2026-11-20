//! Compile-pass coverage for the handler builders' `const` setter chains.
//!
//! Each `const fn` below calls one public setter on a builder passed in as an
//! argument, so the fixture stops compiling the moment a setter loses its
//! `const` qualifier. A run-time test cannot detect that: a `const fn` that
//! quietly stops being one still passes every behavioural test.
//!
//! The chain cannot start from `new(..)` inside a `const` item. All three
//! constructors allocate (`PathBuf` or `String`) and are therefore ordinary
//! `fn`s, so the builder arrives as a parameter instead. `FileHandlerBuilder`
//! additionally gets its `with_capacity` and `with_flush_after_records` from
//! `builder_methods!`, which emits non-`const` methods; only its hand-written
//! `with_overflow_policy` is pinned here. The crate-private
//! `FormatterId::Default.as_str()` cannot be named by an external fixture at
//! all, so `src/handlers/formatter_id.rs` carries that in-crate pin.

use std::num::NonZeroU64;

use _femtologging_rs::{
    FileHandlerBuilder, OverflowPolicy, RotatingFileHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
};

/// The overflow policy every chain applies.
const POLICY: OverflowPolicy =
    OverflowPolicy::Timeout(std::time::Duration::from_millis(250));

/// A flush threshold the builders accept, established in const context.
const FLUSH_AFTER_RECORDS: NonZeroU64 = match NonZeroU64::new(8) {
    Some(value) => value,
    None => panic!("8 is non-zero"),
};

const fn file_builder_chain(builder: FileHandlerBuilder) -> FileHandlerBuilder {
    builder.with_overflow_policy(POLICY)
}

const fn rotating_builder_chain(
    builder: RotatingFileHandlerBuilder,
) -> RotatingFileHandlerBuilder {
    builder
        .with_capacity(64)
        .with_flush_after_records(FLUSH_AFTER_RECORDS)
        .with_overflow_policy(POLICY)
}

const fn timed_rotating_builder_chain(
    builder: TimedRotatingFileHandlerBuilder,
) -> TimedRotatingFileHandlerBuilder {
    builder
        .with_capacity(64)
        .with_flush_after_records(FLUSH_AFTER_RECORDS)
        .with_overflow_policy(POLICY)
}

fn main() {
    // The const fns are the assertion; naming them keeps them used.
    let _: fn(FileHandlerBuilder) -> FileHandlerBuilder = file_builder_chain;
    let _: fn(RotatingFileHandlerBuilder) -> RotatingFileHandlerBuilder = rotating_builder_chain;
    let _: fn(TimedRotatingFileHandlerBuilder) -> TimedRotatingFileHandlerBuilder =
        timed_rotating_builder_chain;
}