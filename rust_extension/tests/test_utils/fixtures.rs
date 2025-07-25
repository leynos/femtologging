//! Test fixtures that provide `(Arc<Mutex<Vec<u8>>>, FemtoStreamHandler)` pairs for
//! integration and property tests. These helpers wrap a shared in-memory buffer
//! so that handlers can be exercised without touching the filesystem.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::shared_buffer::std::SharedBuf;
use _femtologging_rs::{
    rate_limited_warner::RateLimitedWarner, DefaultFormatter, FemtoStreamHandler,
    StreamHandlerConfig,
};
use rstest::fixture;

fn fresh_buffer() -> Arc<Mutex<Vec<u8>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// Return a handler with a fresh in-memory buffer using the default configuration.
#[fixture]
pub fn handler_tuple() -> (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler) {
    let buffer = fresh_buffer();
    let handler = FemtoStreamHandler::new(SharedBuf(Arc::clone(&buffer)), DefaultFormatter);
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
) -> (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler) {
    let buffer = fresh_buffer();
    let handler = FemtoStreamHandler::with_test_config(
        SharedBuf(Arc::clone(&buffer)),
        DefaultFormatter,
        StreamHandlerConfig::default()
            .with_capacity(1)
            .with_timeout(Duration::from_millis(50))
            .with_warner(RateLimitedWarner::new(warn_interval)),
    );
    (buffer, handler)
}
