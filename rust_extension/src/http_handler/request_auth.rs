//! Authentication and header decoration for outgoing HTTP requests.
//!
//! Every GET and POST request is decorated the same way, so the parts that
//! turn configuration into headers live here rather than in the worker loop.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};

use super::config::{AuthConfig, HTTPHandlerConfig};

/// Add the configured authentication header to `req`.
///
/// A bearer token is sent as-is. Basic credentials are combined and encoded
/// per request because `ureq` owns each request value.
pub(super) fn apply_auth(config: &HTTPHandlerConfig, req: ureq::Request) -> ureq::Request {
    match &config.auth {
        AuthConfig::None => req,
        AuthConfig::Basic { username, password } => {
            let credentials = format!("{username}:{password}");
            let encoded = base64_encode(credentials.as_bytes());
            req.set("Authorization", &format!("Basic {encoded}"))
        }
        AuthConfig::Bearer { token } => req.set("Authorization", &format!("Bearer {token}")),
    }
}

/// Apply every configured custom header to `req`.
pub(super) fn apply_headers(config: &HTTPHandlerConfig, mut req: ureq::Request) -> ureq::Request {
    for (key, value) in &config.headers {
        req = req.set(key, value);
    }
    req
}

/// Base64-encode a byte slice for Basic authentication.
pub(super) fn base64_encode(input: &[u8]) -> String {
    BASE64_STANDARD.encode(input)
}

#[cfg(test)]
#[path = "request_auth_tests.rs"]
mod tests;
