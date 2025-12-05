//! Test fixtures that provide `(SharedBytes, FemtoStreamHandler)` pairs for
//! integration and property tests. These helpers wrap a shared in-memory buffer
//! so that handlers can be exercised without touching the file system.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use _femtologging_rs::{
    DefaultFormatter, FemtoStreamHandler, StreamHandlerConfig,
    rate_limited_warner::RateLimitedWarner,
};
use rstest::fixture;

use super::shared_buffer::std::SharedBuf;

/// Shared in-memory byte buffer.
type SharedBytes = Arc<Mutex<Vec<u8>>>;

/// Return a new shared in-memory buffer wrapped in `SharedBytes`.
#[must_use]
fn fresh_buffer() -> SharedBytes {
    Arc::new(Mutex::new(Vec::new()))
}

/// Return a handler with a fresh in-memory buffer using the default configuration.
#[fixture]
pub fn handler_tuple() -> (SharedBytes, FemtoStreamHandler) {
    let buffer = fresh_buffer();
    let handler = FemtoStreamHandler::new(SharedBuf::new(Arc::clone(&buffer)), DefaultFormatter);
    (buffer, handler)
}

/// Return a handler backed by a shared buffer with a small capacity and
/// short timeout.
///
/// # Arguments
/// * `warn_interval` – the minimum duration between successive rate-limited
///   warnings emitted by the handler.
#[fixture]
pub fn handler_tuple_custom(
    #[default(Duration::from_secs(5))] warn_interval: Duration,
) -> (SharedBytes, FemtoStreamHandler) {
    let buffer = fresh_buffer();
    let handler = FemtoStreamHandler::with_test_config(
        SharedBuf::new(Arc::clone(&buffer)),
        DefaultFormatter,
        StreamHandlerConfig::default()
            .with_capacity(1)
            .with_timeout(Duration::from_millis(50))
            .with_warner(RateLimitedWarner::new(warn_interval)),
    );
    (buffer, handler)
}
