//! Compile-fail coverage for the `#[must_use]` rotating builder setters.
//!
//! The setters consume and return the builder, so the returned value is the
//! only copy carrying the requested threshold. Discarding it is almost always a
//! mistake, which is why they are `#[must_use]`.

#![deny(unused_must_use)]

use _femtologging_rs::{RotatingFileHandlerBuilder, TimedRotatingFileHandlerBuilder};

fn main() {
    RotatingFileHandlerBuilder::new("unused_max_bytes.log").with_max_bytes(1024);
    TimedRotatingFileHandlerBuilder::new("unused_backup_count.log").with_backup_count(3);
}