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
    pub capacity: usize,
    pub connect_timeout: Duration,
    pub write_timeout: Duration,
    pub max_frame_size: usize,
    pub transport: SocketTransport,
    pub backoff: BackoffPolicy,
    pub warn_interval: Duration,
}

/// Provide defaults that favour local development whilst encouraging
/// production callers to override the transport via the builder APIs.
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
    pub base: Duration,
    pub cap: Duration,
    pub reset_after: Duration,
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
