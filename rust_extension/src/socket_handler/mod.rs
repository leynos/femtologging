//! Socket-based logging handler implementation.
//!
//! This module defines `FemtoSocketHandler`, a handler that serialises
//! [`FemtoLogRecord`](crate::log_record::FemtoLogRecord) values into
//! MessagePack frames and forwards them to a remote socket. The consumer thread
//! maintains the underlying connection, transparently reconnects using
//! exponential backoff, and exposes builder-friendly configuration points for
//! transport selection, timeouts, and TLS options.

pub(crate) mod backoff;
mod config;
mod handler;
mod serialise;
mod transport;
mod worker;

#[cfg(test)]
mod tests;

pub use config::{BackoffPolicy, SocketHandlerConfig};
pub use handler::FemtoSocketHandler;
pub use transport::{SocketTransport, TcpTransport, TlsOptions, UnixTransport};
