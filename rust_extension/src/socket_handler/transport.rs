//! Transport primitives for the socket handler.

use std::{
    io::{self, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::PathBuf,
    time::Duration,
};

use native_tls::{TlsConnector, TlsStream};

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
    PlainTcp(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    #[cfg(unix)]
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

fn connect_tcp(config: &TcpTransport, timeout: Duration) -> io::Result<TcpStream> {
    let addrs = config.socket_addrs()?;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::TimedOut {
                    continue;
                }
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("unable to connect to {}:{}", config.host, config.port),
    ))
}

/// Establish a socket connection using the provided transport definition.
pub fn connect_transport(
    transport: &SocketTransport,
    connect_timeout: Duration,
) -> io::Result<ActiveConnection> {
    match transport {
        SocketTransport::Tcp(config) => {
            let stream = connect_tcp(config, connect_timeout)?;
            if let Some(tls) = &config.tls {
                let connector = tls.connector()?;
                stream.set_read_timeout(Some(connect_timeout))?;
                stream.set_write_timeout(Some(connect_timeout))?;
                let stream = connector
                    .connect(&tls.domain, stream)
                    .map_err(io::Error::other)?;
                let tcp_ref = stream.get_ref();
                tcp_ref.set_read_timeout(None)?;
                tcp_ref.set_write_timeout(None)?;
                Ok(ActiveConnection::Tls(Box::new(stream)))
            } else {
                Ok(ActiveConnection::PlainTcp(stream))
            }
        }
        SocketTransport::Unix(config) => {
            #[cfg(unix)]
            {
                let stream = UnixStream::connect(&config.path)?;
                Ok(ActiveConnection::Unix(stream))
            }
            #[cfg(not(unix))]
            {
                let _ = (config, connect_timeout);
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "unix domain sockets are not supported on this platform",
                ))
            }
        }
    }
}
