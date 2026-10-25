//! Transport primitives for the socket handler.
//!
//! Every connection attempt here takes the handler's [`ShutdownSignal`] as well
//! as a timeout, and consults it between the steps of a connection rather than
//! only before them. One attempt is several blocking calls - one per resolved
//! address, then the TLS handshake - and a budget that only bounded the first
//! would be spent by it while the rest ran on the configuration's timeouts.

use std::{
    io::{self, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::PathBuf,
    time::{Duration, Instant},
};

use native_tls::{TlsConnector, TlsStream};

use crate::shutdown_signal::ShutdownSignal;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// Transport targeted by the socket handler.
#[derive(Clone, Debug)]
pub enum SocketTransport {
    /// TCP transport with optional TLS.
    Tcp(TcpTransport),
    /// Unix domain socket transport.
    Unix(UnixTransport),
}

/// TCP transport configuration.
#[derive(Clone, Debug)]
pub struct TcpTransport {
    /// Hostname or IP address to connect to.
    pub host: String,
    /// TCP port number.
    pub port: u16,
    /// Optional TLS configuration.
    pub tls: Option<TlsOptions>,
}

impl TcpTransport {
    /// Resolve the configured host and port into candidate socket addresses.
    ///
    /// Example: a hostname with IPv4 and IPv6 records returns both candidates.
    fn socket_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map(|iter| iter.collect())
    }
}

/// Unix domain socket configuration.
#[derive(Clone, Debug)]
pub struct UnixTransport {
    /// Path to the socket file.
    pub path: PathBuf,
}

/// TLS connection options.
#[derive(Clone, Debug)]
pub struct TlsOptions {
    /// Domain name presented during the TLS handshake.
    pub domain: String,
    /// Skip certificate validation when true (intended for tests).
    pub insecure_skip_verify: bool,
}

impl TlsOptions {
    /// Build a TLS connector using this option set.
    ///
    /// Example: `insecure_skip_verify` permits the test peer's self-signed certificate.
    fn connector(&self) -> io::Result<TlsConnector> {
        let mut builder = TlsConnector::builder();
        if self.insecure_skip_verify {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        builder.build().map_err(io::Error::other)
    }
}

/// Active socket connection state.
pub enum ActiveConnection {
    /// A connected TCP stream without TLS.
    PlainTcp(TcpStream),
    /// A connected TCP stream protected by TLS.
    Tls(Box<TlsStream<TcpStream>>),
    #[cfg(unix)]
    /// A connected Unix domain socket.
    Unix(UnixStream),
}

impl ActiveConnection {
    /// Update the write timeout for the underlying socket.
    pub fn set_write_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        match self {
            ActiveConnection::PlainTcp(stream) => stream.set_write_timeout(Some(timeout)),
            ActiveConnection::Tls(stream) => stream.get_ref().set_write_timeout(Some(timeout)),
            #[cfg(unix)]
            ActiveConnection::Unix(stream) => stream.set_write_timeout(Some(timeout)),
        }
    }

    /// Write a full buffer to the socket.
    pub fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self {
            ActiveConnection::PlainTcp(stream) => stream.write_all(buf),
            ActiveConnection::Tls(stream) => stream.write_all(buf),
            #[cfg(unix)]
            ActiveConnection::Unix(stream) => stream.write_all(buf),
        }
    }

    /// Flush the underlying writer.
    pub fn flush(&mut self) -> io::Result<()> {
        match self {
            ActiveConnection::PlainTcp(stream) => stream.flush(),
            ActiveConnection::Tls(stream) => stream.flush(),
            #[cfg(unix)]
            ActiveConnection::Unix(stream) => stream.flush(),
        }
    }
}

/// What one step of a connection attempt may spend.
///
/// `None` once the attempt's own deadline has passed, or the handler's
/// shutdown budget is spent - whichever comes first. Both are ceilings on the
/// same call, so neither can outlast the other.
fn step_budget(signal: &ShutdownSignal, deadline: Instant) -> Option<Duration> {
    let left = deadline.checked_duration_since(Instant::now())?;
    signal.clamp(left).filter(|budget| !budget.is_zero())
}

/// Connect to the first address that answers, within one shared budget.
///
/// The budget is a deadline over the whole sequence, not a per-address
/// timeout. An earlier version handed each address the full `connect_timeout`,
/// so a host resolving to several unresponsive addresses could spend that
/// timeout once per address with nothing bounding the total.
fn connect_tcp(
    config: &TcpTransport,
    deadline: Instant,
    signal: &ShutdownSignal,
) -> io::Result<TcpStream> {
    let addrs = config.socket_addrs()?;
    let mut last_error = None;
    for addr in addrs {
        let Some(left) = step_budget(signal, deadline) else {
            break;
        };
        match TcpStream::connect_timeout(&addr, left) {
            Ok(stream) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(err) => {
                // Remembered rather than dropped: which address was tried, and
                // how it failed, is the only thing that explains a connection
                // error to someone reading the log.
                last_error = Some(err);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("unable to connect to {}:{}", config.host, config.port),
        )
    }))
}

/// Establish a socket connection using the provided transport definition.
///
/// `connect_timeout` bounds the whole attempt; `signal` bounds each step of it
/// by what remains of the handler's shutdown budget, and stops the attempt
/// outright once that budget is spent.
pub fn connect_transport(
    transport: &SocketTransport,
    connect_timeout: Duration,
    signal: &ShutdownSignal,
) -> io::Result<ActiveConnection> {
    let deadline = Instant::now() + connect_timeout;
    match transport {
        SocketTransport::Tcp(config) => connect_tcp_transport(config, deadline, signal),
        SocketTransport::Unix(config) => connect_unix_transport(config, signal),
    }
}

/// Connect a TCP transport, performing a TLS handshake when configured.
///
/// Example: plain TCP returns a `PlainTcp` connection without a handshake.
fn connect_tcp_transport(
    config: &TcpTransport,
    deadline: Instant,
    signal: &ShutdownSignal,
) -> io::Result<ActiveConnection> {
    match &config.tls {
        Some(tls) => connect_tls(config, tls, deadline, signal),
        None => connect_tcp(config, deadline, signal).map(ActiveConnection::PlainTcp),
    }
}

/// Connect a Unix socket unless shutdown has already aborted the attempt.
///
/// Example: a live signal connects to the configured socket path.
#[cfg(unix)]
fn connect_unix_transport(
    config: &UnixTransport,
    signal: &ShutdownSignal,
) -> io::Result<ActiveConnection> {
    if signal.is_aborted() {
        Err(budget_spent())
    } else {
        UnixStream::connect(&config.path).map(ActiveConnection::Unix)
    }
}

/// Return the platform's unsupported error for Unix socket transports.
///
/// Example: this path reports `Unsupported` on Windows.
#[cfg(not(unix))]
fn connect_unix_transport(
    config: &UnixTransport,
    signal: &ShutdownSignal,
) -> io::Result<ActiveConnection> {
    let _ = (config, signal);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unix domain sockets are not supported on this platform",
    ))
}

/// Connect, then shake hands within what is left of the same budget.
///
/// The handshake inherited the budget by way of the socket timeouts, which is
/// why they are set here and cleared again on success. Left in place they would
/// become the connection's timeouts, and every later write would fail on a
/// deadline that belonged to the connect.
fn connect_tls(
    config: &TcpTransport,
    tls: &TlsOptions,
    deadline: Instant,
    signal: &ShutdownSignal,
) -> io::Result<ActiveConnection> {
    let stream = connect_tcp(config, deadline, signal)?;
    let Some(left) = step_budget(signal, deadline) else {
        return Err(budget_spent());
    };
    let connector = tls.connector()?;
    stream.set_read_timeout(Some(left))?;
    stream.set_write_timeout(Some(left))?;
    let stream = connector
        .connect(&tls.domain, stream)
        .map_err(io::Error::other)?;
    let tcp_ref = stream.get_ref();
    tcp_ref.set_read_timeout(None)?;
    tcp_ref.set_write_timeout(None)?;
    Ok(ActiveConnection::Tls(Box::new(stream)))
}

/// The error a spent budget produces.
///
/// `TimedOut` rather than a bespoke kind: to the worker this is one more
/// connection that did not happen in the time allowed, which is what its
/// backoff and warning paths already handle.
fn budget_spent() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "shutdown budget spent before the connection was established",
    )
}
