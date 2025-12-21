//! Configuration structures consumed by the HTTP handler lifecycle.
//!
//! `HTTPHandlerBuilder` constructs these values before passing them to
//! [`FemtoHTTPHandler`](super::FemtoHTTPHandler) for runtime use.

use std::collections::HashMap;
use std::time::Duration;

use crate::rate_limited_warner::DEFAULT_WARN_INTERVAL;
use crate::socket_handler::BackoffPolicy;

/// Default bounded channel capacity used by the handler.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
/// Default connection timeout applied when establishing HTTP connections.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default write/request timeout applied to HTTP requests.
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP methods supported by the handler.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HTTPMethod {
    /// HTTP GET method - appends payload to URL query string.
    GET,
    /// HTTP POST method - sends payload in request body.
    #[default]
    POST,
}

impl HTTPMethod {
    /// Convert to the string representation used by ureq.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GET => "GET",
            Self::POST => "POST",
        }
    }
}

/// Authentication configuration for HTTP requests.
#[derive(Clone, Debug, Default)]
pub enum AuthConfig {
    /// No authentication.
    #[default]
    None,
    /// HTTP Basic authentication with username and password.
    Basic { username: String, password: String },
    /// Bearer token authentication.
    Bearer { token: String },
}

/// Serialization format for log records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SerializationFormat {
    /// URL-encoded form data (CPython `logging.HTTPHandler` default).
    #[default]
    UrlEncoded,
    /// JSON serialization for modern HTTP APIs.
    Json,
}

/// Configuration object describing how to construct a
/// [`FemtoHTTPHandler`](super::FemtoHTTPHandler).
#[derive(Clone, Debug)]
pub struct HTTPHandlerConfig {
    /// Bounded channel capacity for the producer-consumer queue.
    pub capacity: usize,
    /// Target URL for HTTP requests.
    pub url: String,
    /// HTTP method (GET or POST).
    pub method: HTTPMethod,
    /// Authentication configuration.
    pub auth: AuthConfig,
    /// Additional HTTP headers to include in requests.
    pub headers: HashMap<String, String>,
    /// Timeout for establishing connections.
    pub connect_timeout: Duration,
    /// Timeout for sending requests (write timeout).
    pub write_timeout: Duration,
    /// Exponential backoff policy for retries.
    pub backoff: BackoffPolicy,
    /// Interval between rate-limited warnings.
    pub warn_interval: Duration,
    /// Serialization format for log records.
    pub format: SerializationFormat,
    /// Optional list of fields to include in serialization.
    /// When `None`, all fields are included.
    pub record_fields: Option<Vec<String>>,
}

impl Default for HTTPHandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            url: String::new(),
            method: HTTPMethod::default(),
            auth: AuthConfig::default(),
            headers: HashMap::new(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            write_timeout: DEFAULT_WRITE_TIMEOUT,
            backoff: BackoffPolicy::default(),
            warn_interval: DEFAULT_WARN_INTERVAL,
            format: SerializationFormat::default(),
            record_fields: None,
        }
    }
}
