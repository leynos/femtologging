//! Trust-selection contracts for the HTTP transport's TLS provider.
//!
//! The migration from ureq 2 to 3 had to keep native-tls validating against
//! the host's trust store. A fixture certificate is therefore the only way to
//! tell the two interesting answers apart without touching system trust:
//! explicitly trusting the fixture must succeed, and the production roots must
//! refuse it. Both tests talk to a listener on loopback and read no
//! environment.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use native_tls::{Identity, TlsAcceptor};
use ureq::tls::{Certificate, RootCerts};

use crate::http_handler::config::HTTPHandlerConfig;
use crate::http_handler::response::ResponseClass;
use crate::http_handler::transport::execute_with_roots;

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
        let mut sink = [0_u8; 4096];
        let _ = stream.read(&mut sink);
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

/// Scenario: the client is told to trust exactly the fixture certificate.
///
/// Invariant: the native-tls provider completes the handshake and the record is
/// delivered. This proves the agent uses `TlsProvider::NativeTls` and honours a
/// `RootCerts::Specific` trust anchor rather than ignoring the setting.
#[test]
fn a_specific_root_lets_native_tls_trust_the_fixture() {
    let ca = test_ca();
    let (port, server) = spawn_tls_server(&ca).expect("spawn TLS fixture server");

    let class = execute_with_roots(&https_config(port), ca.roots(), "msg=tls")
        .expect("the fixture certificate is the configured trust anchor");

    assert_eq!(class, ResponseClass::Success);
    server.join().expect("TLS fixture server should finish");
}

/// Scenario: the same fixture server, but trust left at the production roots.
///
/// Invariant: the handshake fails. A migration that let ureq 3 default to
/// bundled WebPKI roots, or one that silently disabled verification, would not
/// produce this error - the first would be indistinguishable from success here
/// only if the fixture leaked into that store, and the second would succeed.
#[test]
fn production_roots_refuse_an_unknown_certificate() {
    let ca = test_ca();
    let (port, server) = spawn_tls_server(&ca).expect("spawn TLS fixture server");

    let err = execute_with_roots(&https_config(port), RootCerts::PlatformVerifier, "msg=tls")
        .expect_err("the host trust store must not accept a throwaway certificate");

    assert!(
        !err.is_empty(),
        "a rejected certificate must surface as a transport error"
    );
    server.join().expect("TLS fixture server should finish");
}
