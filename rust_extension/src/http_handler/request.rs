//! Request construction for the HTTP worker.
//!
//! The URL, authentication header and custom headers are a pure function of
//! the handler configuration, so they live apart from the worker loop that
//! owns retries and timing. Splitting them out keeps the worker module within
//! the repository's file-size limit and makes the wire shape reviewable on its
//! own.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ureq::{Agent, Request};

use super::config::{AuthConfig, HTTPHandlerConfig, SerializationFormat};

/// Build a GET request; the payload follows any existing query data.
pub(super) fn build_get_request(
    config: &HTTPHandlerConfig,
    payload: &str,
    agent: &Agent,
) -> Result<ureq::Response, Box<ureq::Error>> {
    let url = if config.url.contains('?') {
        format!("{}&{}", config.url, payload)
    } else {
        format!("{}?{}", config.url, payload)
    };
    let mut req = agent.get(&url);
    req = apply_auth(config, req);
    req = apply_headers(config, req);
    req.call().map_err(Box::new)
}

/// Build a POST request; JSON format sends an `application/json` body.
pub(super) fn build_post_request(
    config: &HTTPHandlerConfig,
    payload: &str,
    agent: &Agent,
) -> Result<ureq::Response, Box<ureq::Error>> {
    let mut req = agent.post(&config.url);
    req = apply_auth(config, req);
    req = apply_headers(config, req);

    let content_type = match config.format {
        SerializationFormat::UrlEncoded => "application/x-www-form-urlencoded",
        SerializationFormat::Json => "application/json",
    };
    req = req.set("Content-Type", content_type);
    req.send_string(payload).map_err(Box::new)
}

/// Add authentication; bearer configuration sets an `Authorization` header.
fn apply_auth(config: &HTTPHandlerConfig, req: Request) -> Request {
    match &config.auth {
        AuthConfig::None => req,
        AuthConfig::Basic { username, password } => {
            let credentials = format!("{}:{}", username, password);
            let encoded = base64_encode(credentials.as_bytes());
            req.set("Authorization", &format!("Basic {}", encoded))
        }
        AuthConfig::Bearer { token } => req.set("Authorization", &format!("Bearer {}", token)),
    }
}

/// Apply every configured custom header to the request.
fn apply_headers(config: &HTTPHandlerConfig, mut req: Request) -> Request {
    for (key, value) in &config.headers {
        req = req.set(key, value);
    }
    req
}

/// Base64-encode a byte slice for Basic auth.
pub(super) fn base64_encode(input: &[u8]) -> String {
    BASE64_STANDARD.encode(input)
}
