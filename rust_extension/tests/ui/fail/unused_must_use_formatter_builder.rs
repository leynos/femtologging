//! Compile-fail coverage for the `#[must_use]` formatter builder.
//!
//! `FormatterBuilder` is immutable: every setter consumes the builder and
//! returns a new one, and the setter carries `#[must_use]`. A caller that
//! discards the result loses the whole chain, so the deny below turns that
//! silence into a build error.

#![deny(unused_must_use)]

use _femtologging_rs::FormatterBuilder;

fn main() {
    FormatterBuilder::new();
    FormatterBuilder::new().with_format("{message}");
    FormatterBuilder::new().with_datefmt("%Y-%m-%d");
}
