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
mod response;
mod serialize;
mod transport;
mod url_encoding;
mod worker;

#[cfg(test)]
mod tests;
// The TLS fixture is an rcgen ECDSA certificate. native-tls's Schannel backend
// imports PKCS#8 identities through an RSA provider, so the fixture cannot be
// loaded on Windows, and rcgen's ring backend cannot generate an RSA key to
// replace it. Only the fixture is platform-bound; the transport it exercises
// is not.
#[cfg(all(test, not(target_os = "windows")))]
mod tls_tests;

pub use config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat};
pub use handler::FemtoHTTPHandler;

pub(crate) use transport::validate_request_headers;
