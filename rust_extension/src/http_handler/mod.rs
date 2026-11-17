//! HTTP-based logging handler implementation.
//!
//! This module defines [`FemtoHTTPHandler`], a handler that serializes
//! [`FemtoLogRecord`](crate::log_record::FemtoLogRecord) values and
//! forwards them to an HTTP endpoint. The consumer thread maintains
//! the underlying HTTP client, handles reconnection with exponential
//! backoff, and supports configurable authentication and timeouts.
//!
//! # Serialization Formats
//!
//! Two serialization formats are supported:
//!
//! - **URL-encoded** (default): Produces `application/x-www-form-urlencoded`
//!   payloads matching CPython's `logging.HTTPHandler` format.
//! - **JSON**: Produces `application/json` payloads for modern HTTP APIs.
//!
//! # Retry Semantics
//!
//! The handler classifies HTTP responses for retry decisions:
//!
//! - **2xx**: Success - reset backoff state.
//! - **429 (Too Many Requests)**: Retryable - apply backoff and retry.
//! - **5xx**: Retryable - apply backoff and retry.
//! - **4xx (except 429)**: Permanent failure - drop record without retry.
//! - **Network errors**: Retryable - apply backoff and retry.

mod config;
mod filtered;
mod handler;
mod queue;
mod record;
mod request_auth;
mod response;
mod serialize;
// The bridges suspend this marker as well as the worker setting it, so the
// module is crate-visible even though it belongs to the HTTP adapter.
pub(crate) mod transport_origin;
mod url_encoding;
mod worker;

#[cfg(test)]
mod tests;

pub use config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat};
pub use handler::FemtoHTTPHandler;

/// What asking the worker to stop established about joining it.
///
/// The handler's close path lives in `handler.rs`; this verdict is shared
/// vocabulary for that decision rather than part of the public surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShutdownOutcome {
    /// The worker acknowledged, or had already left its loop. The join is
    /// bounded and worth doing, because it is also what surfaces a panic.
    Finished,
    /// The command could not be queued, or no acknowledgement arrived, within
    /// `flush_timeout`. The worker is busy on something whose length
    /// `flush_timeout` does not describe.
    TimedOut,
}
