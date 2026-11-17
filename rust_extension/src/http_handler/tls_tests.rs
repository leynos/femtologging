//! Trust contracts for the HTTP transport's TLS provider.
//!
//! The migration from ureq 2 to 3 had to keep validating certificates against
//! the WebPKI roots bundled into the binary, which is what ureq 2's default
//! Rustls connector used. A fixture certificate is therefore the only way to
//! tell the interesting answers apart without touching system trust: explicitly
//! trusting the fixture must succeed, and the production agent must refuse it.
//! Every test talks to a listener on loopback and reads no environment.
//!
//! The fixture server uses native-tls, but that says nothing about the client:
//! the server is only a convenient way to present a throwaway certificate.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use native_tls::{Identity, TlsAcceptor, TlsStream};
use ureq::tls::{Certificate, RootCerts, TlsProvider};

use crate::http_handler::config::HTTPHandlerConfig;
use crate::http_handler::response::ResponseClass;
use crate::http_handler::transport::{build_agent, execute, execute_with_roots};

/// A throwaway certificate authority and the material a server needs for it.
struct TestCa {
    /// PEM certificate, as `native_tls::Identity` expects it.
    cert_pem: String,
    /// PKCS#8 private key of the same certificate.
    key_pem: String,
}

impl TestCa {
    /// Trust anchor for a client that should accept the fixture.
    fn roots(&self) -> RootCerts {
        let cert = Certificate::from_pem(self.cert_pem.as_bytes()).expect("parse fixture cert");
        RootCerts::new_with_certs(&[cert])
    }
}

/// Generate a self-signed certificate for `localhost`.
fn test_ca() -> TestCa {
    let key = rcgen::KeyPair::generate().expect("generate fixture key");
    let params = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .expect("build certificate parameters");
    let cert = params
        .self_signed(&key)
        .expect("self-sign the fixture certificate");
    TestCa {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    }
}

/// Read one complete HTTP request so the fixture does not close on unread
/// bytes.
///
/// A TLS read returns the data of one record, so a single read can leave the
/// body queued. Closing the stream with unread bytes can reset the connection
/// before the client has read the response, which would make the trusting test
/// fail intermittently.
fn read_full_request(stream: &mut TlsStream<TcpStream>) -> std::io::Result<()> {
    stream
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
        if let Some(value) = header_value(&line, "content-length") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body)
}

/// The value of `name` if `line` is that header, compared case-insensitively.
fn header_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let (field, value) = line.split_once(':')?;
    field.eq_ignore_ascii_case(name).then_some(value)
}

/// Serve one HTTPS request and answer 200.
fn spawn_tls_server(ca: &TestCa) -> std::io::Result<(u16, thread::JoinHandle<()>)> {
    let identity = Identity::from_pkcs8(ca.cert_pem.as_bytes(), ca.key_pem.as_bytes())
        .expect("native-tls accepts the fixture identity");
    let acceptor = TlsAcceptor::new(identity).expect("build TLS acceptor");
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let handle = thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let Ok(mut stream) = acceptor.accept(stream) else {
            return;
        };
        let _ = read_full_request(&mut stream);
        let _ =
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    Ok((port, handle))
}

/// Configuration that posts to the fixture over HTTPS.
fn https_config(port: u16) -> HTTPHandlerConfig {
    HTTPHandlerConfig {
        url: format!("https://localhost:{port}/log"),
        connect_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        ..Default::default()
    }
}

/// Scenario: the production agent is inspected before it sends anything.
///
/// Invariant: it selects Rustls and the WebPKI roots, with verification left
/// on. This is the policy ureq 2 actually applied, and pinning it here keeps a
/// change of ureq's defaults, or a later edit to the builder, from silently
/// changing which certificates the handler trusts.
#[test]
fn the_production_agent_trusts_the_bundled_webpki_roots() {
    let agent = build_agent(Duration::from_secs(5), Duration::from_secs(5));

    let tls = agent.config().tls_config();

    assert_eq!(
        tls.provider(),
        TlsProvider::Rustls,
        "the handler must not switch to a TLS library with different trust rules"
    );
    assert!(
        matches!(tls.root_certs(), RootCerts::WebPki),
        "the production roots must be the bundled WebPKI set: {:?}",
        tls.root_certs()
    );
    assert!(
        !tls.disable_verification(),
        "certificate verification must never be disabled in production"
    );
}

/// Scenario: the client is told to trust exactly the fixture certificate.
///
/// Invariant: the handshake completes and the record is delivered. This proves
/// the agent honours a `RootCerts::Specific` trust anchor rather than ignoring
/// the setting, which is what makes the refusing test below meaningful.
#[test]
fn a_specific_root_lets_the_client_trust_the_fixture() {
    let ca = test_ca();
    let (port, server) = spawn_tls_server(&ca).expect("spawn TLS fixture server");

    let class = execute_with_roots(&https_config(port), ca.roots(), "msg=tls")
        .expect("the fixture certificate is the configured trust anchor");

    assert_eq!(class, ResponseClass::Success);
    server.join().expect("TLS fixture server should finish");
}

/// Scenario: the same fixture server, reached through the production agent.
///
/// Invariant: the handshake fails with a transport error. An agent that
/// silently disabled verification would succeed, and one that trusted the
/// host's trust store rather than the bundled roots would need the fixture to
/// be installed there to succeed - which it is not.
#[test]
fn the_production_agent_refuses_an_unknown_certificate() {
    let ca = test_ca();
    let (port, server) = spawn_tls_server(&ca).expect("spawn TLS fixture server");
    let config = https_config(port);
    let agent = build_agent(config.connect_timeout, config.write_timeout);

    let err = execute(&config, &agent, "msg=tls")
        .expect_err("the production roots must not accept a throwaway certificate");

    assert!(
        !err.is_empty(),
        "a rejected certificate must surface as a transport error"
    );
    server.join().expect("TLS fixture server should finish");
}
