//! Test fixtures that provide `(SharedBytes, FemtoStreamHandler)` pairs for
//! integration and property tests. These helpers wrap a shared in-memory buffer
//! so that handlers can be exercised without touching the file system.

use std::sync::{Arc, Mutex};

use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler};
use rstest::fixture;

use super::shared_buffer::std::SharedBuf;

/// Shared in-memory byte buffer.
type SharedBytes = Arc<Mutex<Vec<u8>>>;

/// Return a new shared in-memory buffer wrapped in `SharedBytes`.
#[must_use]
fn fresh_buffer() -> SharedBytes {
    Arc::new(Mutex::new(Vec::new()))
}

/// Return a default-configured handler that writes into `buffer`.
///
/// Use this when several handlers must share one buffer; otherwise prefer the
/// `handler_tuple` fixture, which supplies a fresh buffer of its own.
#[must_use]
pub fn stream_handler_for(buffer: &SharedBytes) -> FemtoStreamHandler {
    FemtoStreamHandler::new(SharedBuf::new(Arc::clone(buffer)), DefaultFormatter)
}

/// Return a handler with a fresh in-memory buffer using the default configuration.
#[fixture]
pub fn handler_tuple() -> (SharedBytes, FemtoStreamHandler) {
    let buffer = fresh_buffer();
    let handler = stream_handler_for(&buffer);
    (buffer, handler)
}
