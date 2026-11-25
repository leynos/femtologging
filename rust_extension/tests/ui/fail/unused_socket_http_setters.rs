//! Compile-fail coverage for the `#[must_use]` socket and HTTP setters.
//!
//! Both setters consume and return the builder, so the returned value is the
//! only copy carrying the requested configuration. Discarding it is almost
//! always a mistake, which is why they are `#[must_use]`.

#![deny(unused_must_use)]

use _femtologging_rs::{HTTPHandlerBuilder, SocketHandlerBuilder};

fn main() {
    SocketHandlerBuilder::new().with_capacity(16);
    HTTPHandlerBuilder::new().with_json_format();
}