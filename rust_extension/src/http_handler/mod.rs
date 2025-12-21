//! HTTP-based logging handler implementation.
//!
//! This module defines [`FemtoHTTPHandler`], a handler that serialises
//! [`FemtoLogRecord`](crate::log_record::FemtoLogRecord) values and
//! forwards them to an HTTP endpoint. The consumer thread maintains
//! the underlying HTTP client, handles reconnection with exponential
//! backoff, and supports configurable authentication and timeouts.
//!
//! # Serialisation Formats
//!
//! Two serialisation formats are supported:
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
mod handler;
mod serialise;
mod worker;

#[cfg(test)]
mod tests;

pub use config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat};
pub use handler::FemtoHTTPHandler;
