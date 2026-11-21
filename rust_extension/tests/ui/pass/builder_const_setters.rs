//! Compile-pass coverage for the handler builders' `const` setter chains.
//!
//! Each `const fn` below calls one public setter on a builder passed in as an
//! argument, so the fixture stops compiling the moment a setter loses its
//! `const` qualifier. A run-time test cannot detect that: a `const fn` that
//! quietly stops being one still passes every behavioural test.
//!
//! The chains cannot start from `new(..)` inside a `const` item — every builder
//! constructor is an ordinary `fn` — so the builder arrives as a parameter
//! instead. Three setter families are deliberately absent here:
//!
//! - `FileHandlerBuilder` gets its `with_capacity` and
//!   `with_flush_after_records` from `builder_methods!`, which emits
//!   non-`const` methods; only its handwritten `with_overflow_policy` is
//!   `const`.
//! - `with_backoff` needs a `BackoffOverrides` value, and that type is not
//!   re-exported, so an external fixture cannot name it.
//! - `FormatterId::Default.as_str()` is crate-private for the same reason.
//!
//! The two unpinnable families are covered in-crate instead: the backoff
//! overrides and both `with_backoff` setters in
//! `src/handlers/socket_builder/const_eval_tests.rs`, and the formatter
//! identifier in
//! `src/handlers/formatter_id.rs`.
//!
//! Covered here: `FileHandlerBuilder::with_overflow_policy`; the three `const`
//! setters on each of the rotating and timed-rotating file builders; the four
//! generated scalar setters on `SocketHandlerBuilder`; and
//! `HTTPHandlerBuilder`'s three generated scalar setters plus `with_method` and
//! `with_json_format`.

use std::num::NonZeroU64;

use _femtologging_rs::{
    FileHandlerBuilder, HTTPHandlerBuilder, HTTPMethod, OverflowPolicy,
    RotatingFileHandlerBuilder, SocketHandlerBuilder, TimedRotatingFileHandlerBuilder,
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

const fn socket_builder_chain(builder: SocketHandlerBuilder) -> SocketHandlerBuilder {
    builder
        .with_capacity(64)
        .with_connect_timeout_ms(1)
        .with_write_timeout_ms(1)
        .with_max_frame_size(1)
}

const fn http_builder_chain(builder: HTTPHandlerBuilder) -> HTTPHandlerBuilder {
    builder
        .with_capacity(64)
        .with_connect_timeout_ms(1)
        .with_write_timeout_ms(1)
        .with_method(HTTPMethod::GET)
        .with_json_format()
}

fn main() {
    // The const fns are the assertion; naming them keeps them used.
    let _: fn(FileHandlerBuilder) -> FileHandlerBuilder = file_builder_chain;
    let _: fn(RotatingFileHandlerBuilder) -> RotatingFileHandlerBuilder = rotating_builder_chain;
    let _: fn(TimedRotatingFileHandlerBuilder) -> TimedRotatingFileHandlerBuilder =
        timed_rotating_builder_chain;
    let _: fn(SocketHandlerBuilder) -> SocketHandlerBuilder = socket_builder_chain;
    let _: fn(HTTPHandlerBuilder) -> HTTPHandlerBuilder = http_builder_chain;
}