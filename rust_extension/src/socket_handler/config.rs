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
/// Default duration of healthy writes that resets backoff state.
pub const DEFAULT_BACKOFF_RESET: Duration = Duration::from_secs(30);
/// Default absolute deadline for reconnection attempts.
pub const DEFAULT_BACKOFF_DEADLINE: Duration = Duration::from_secs(120);

/// Configuration object describing how to construct a [`FemtoSocketHandler`](super::FemtoSocketHandler).
#[derive(Clone, Debug)]
pub struct SocketHandlerConfig {
    /// Bounded queue size for records waiting to be sent.
    pub capacity: usize,
    /// How long each connection attempt may take before it fails.
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
    /// Delay before the first retry after a failure. The delay doubles on each further failure.
    pub base: Duration,
    /// Upper bound on the delay between reconnection attempts.
    pub cap: Duration,
    /// Period of healthy writes after which the delay returns to `base`.
    pub reset_after: Duration,
    /// Time after the first failure beyond which reconnection gives up.
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
