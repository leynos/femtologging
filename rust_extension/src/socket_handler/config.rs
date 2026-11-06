//! Configuration structures consumed by the socket handler lifecycle.
//!
//! `SocketHandlerBuilder` constructs these values before passing them to
//! [`FemtoSocketHandler`](super::FemtoSocketHandler) for runtime use.

use std::time::Duration;

use crate::rate_limited_warner::DEFAULT_WARN_INTERVAL;

use super::transport::{SocketTransport, TcpTransport};

/// Default bounded channel capacity used by the handler.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
/// Default connection timeout applied when establishing sockets.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default write timeout applied to socket writes.
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Default maximum payload size (in bytes) accepted by the handler.
pub const DEFAULT_MAX_FRAME_SIZE: usize = 1 << 20; // 1 MiB
/// Default base delay for exponential backoff retries.
pub const DEFAULT_BACKOFF_BASE: Duration = Duration::from_millis(100);
/// Default maximum delay for exponential backoff retries.
pub const DEFAULT_BACKOFF_CAP: Duration = Duration::from_secs(10);
/// Default time since the first failure after which a success resets the backoff state.
pub const DEFAULT_BACKOFF_RESET: Duration = Duration::from_secs(30);
/// Default deadline after which backoff sleeps stop; later records still try to connect.
pub const DEFAULT_BACKOFF_DEADLINE: Duration = Duration::from_secs(120);

/// Configuration object describing how to construct a [`FemtoSocketHandler`](super::FemtoSocketHandler).
#[derive(Clone, Debug)]
pub struct SocketHandlerConfig {
    /// Bounded queue size for records waiting to be sent.
    pub capacity: usize,
    /// Timeout for each TCP connection attempt, applied to each resolved address in turn. It
    /// also bounds each read and write during the TLS handshake. DNS resolution and Unix-socket
    /// connections are not bounded by it.
    pub connect_timeout: Duration,
    /// How long a single socket write may block before it fails.
    pub write_timeout: Duration,
    /// Largest serialized record, in bytes, that will be sent. Larger records are not sent.
    pub max_frame_size: usize,
    /// Destination for records: a TCP host and port, optionally with TLS, or a Unix socket.
    pub transport: SocketTransport,
    /// Backoff policy applied between reconnection attempts.
    pub backoff: BackoffPolicy,
    /// Minimum interval between rate-limited warnings about dropped records and failures.
    pub warn_interval: Duration,
}

/// Provide defaults that favour local development whilst encouraging
/// production callers to override the transport via the builder APIs. The
/// transport defaults to `localhost:9020` so examples and tests work out of
/// the box; real deployments should override it via
/// [`SocketHandlerConfig::with_transport`] or the builder's
/// [`SocketHandlerBuilder::with_tcp`]
/// (crate::handlers::socket_builder::SocketHandlerBuilder::with_tcp)
/// and [`SocketHandlerBuilder::with_unix_path`]
/// (crate::handlers::socket_builder::SocketHandlerBuilder::with_unix_path).
impl Default for SocketHandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            write_timeout: DEFAULT_WRITE_TIMEOUT,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            transport: SocketTransport::Tcp(TcpTransport {
                host: "localhost".into(),
                port: 9020,
                tls: None,
            }),
            backoff: BackoffPolicy::default(),
            warn_interval: DEFAULT_WARN_INTERVAL,
        }
    }
}

impl SocketHandlerConfig {
    /// Override the transport configuration.
    pub fn with_transport(mut self, transport: SocketTransport) -> Self {
        self.transport = transport;
        self
    }
}

/// Exponential backoff policy for reconnection attempts.
#[derive(Clone, Debug)]
pub struct BackoffPolicy {
    /// Upper bound of the jittered delay before the first retry after a failure. Each later
    /// failure in the same failure window doubles this bound, up to `cap`.
    pub base: Duration,
    /// Upper bound on the delay between reconnection attempts.
    pub cap: Duration,
    /// Time since the first failure of the window after which the next successful connection
    /// or write resets the delay to `base`.
    pub reset_after: Duration,
    /// Time since the first failure after which backoff sleeps stop. Later records still
    /// attempt to connect; the worker does not stop reconnecting.
    pub deadline: Duration,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base: DEFAULT_BACKOFF_BASE,
            cap: DEFAULT_BACKOFF_CAP,
            reset_after: DEFAULT_BACKOFF_RESET,
            deadline: DEFAULT_BACKOFF_DEADLINE,
        }
    }
}
