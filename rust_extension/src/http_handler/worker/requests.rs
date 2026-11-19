//! Request construction, execution, and transport-failure classification.
//!
//! Building a request and describing why it failed live together so one
//! attempt can be read end to end. The classification exists because `ureq`'s
//! formatted error text begins with the failed URL, userinfo and query values
//! included, and repeats a rejected header's raw line; a diagnostic derived
//! from it can disclose caller-supplied credentials.

use ureq::Agent;

use crate::http_handler::{
    config::{AuthConfig, HTTPMethod, SerializationFormat},
    response::{ResponseClass, classify_status},
};

use super::{Worker, base64_encode};

impl Worker {
    /// Execute one request; HTTP 503 is classified as retryable.
    ///
    /// Transport failures are reported as an application-owned classification
    /// rather than `ureq`'s formatted error text. That text begins with the
    /// failed URL, including any userinfo and query values, and repeats a
    /// rejected header's raw line; both can carry caller-supplied credentials,
    /// so it is never logged. See [`transport_failure_classification`].
    pub(super) fn execute_request(
        &self,
        payload: &str,
        agent: &Agent,
    ) -> Result<ResponseClass, &'static str> {
        // Note: GET+JSON combination is rejected at build time by HTTPHandlerBuilder.
        let request = match self.config.method {
            HTTPMethod::GET => self.build_get_request(payload, agent),
            HTTPMethod::POST => self.build_post_request(payload, agent),
        };

        match request {
            Ok(response) => Ok(classify_status(response.status())),
            Err(err) => match *err {
                ureq::Error::Status(code, _) => Ok(classify_status(code)),
                ureq::Error::Transport(transport_err) => {
                    Err(transport_failure_classification(&transport_err))
                }
            },
        }
    }

    /// Build a GET request; the payload follows any existing query data.
    fn build_get_request(
        &self,
        payload: &str,
        agent: &Agent,
    ) -> Result<ureq::Response, Box<ureq::Error>> {
        let url = if self.config.url.contains('?') {
            format!("{}&{}", self.config.url, payload)
        } else {
            format!("{}?{}", self.config.url, payload)
        };
        let mut req = agent.get(&url);
        req = self.apply_auth(req);
        req = self.apply_headers(req);
        req.call().map_err(Box::new)
    }

    /// Build a POST request; JSON format sends an `application/json` body.
    fn build_post_request(
        &self,
        payload: &str,
        agent: &Agent,
    ) -> Result<ureq::Response, Box<ureq::Error>> {
        let mut req = agent.post(&self.config.url);
        req = self.apply_auth(req);
        req = self.apply_headers(req);

        let content_type = match self.config.format {
            SerializationFormat::UrlEncoded => "application/x-www-form-urlencoded",
            SerializationFormat::Json => "application/json",
        };
        req = req.set("Content-Type", content_type);
        req.send_string(payload).map_err(Box::new)
    }

    /// Add authentication; bearer configuration sets an `Authorization` header.
    fn apply_auth(&self, req: ureq::Request) -> ureq::Request {
        match &self.config.auth {
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
    fn apply_headers(&self, mut req: ureq::Request) -> ureq::Request {
        for (key, value) in &self.config.headers {
            req = req.set(key, value);
        }
        req
    }
}

/// Describe a transport failure without exposing dependency error text.
///
/// [`ureq::Transport`]'s `Display` output starts with the failed URL, userinfo
/// and query values included, and a rejected header's message repeats the raw
/// header line. Both can carry credentials configured by the caller, so this
/// function classifies the failure instead of formatting it. Every arm returns
/// an application-owned `&'static str`, and the retry-warning path accepts only
/// that type, so a runtime value cannot be interpolated into the warning
/// without failing to compile.
///
/// The returned text completes the warning's `request failed: ` prefix. It
/// includes the `transport error (<label>)` wrapper so the retryable-status
/// message can stay a plain constant beside it.
fn transport_failure_classification(error: &ureq::Transport) -> &'static str {
    match error.kind() {
        ureq::ErrorKind::InvalidUrl => "transport error (invalid URL)",
        ureq::ErrorKind::UnknownScheme => "transport error (unknown scheme)",
        ureq::ErrorKind::Dns => "transport error (DNS lookup failed)",
        ureq::ErrorKind::InsecureRequestHttpsOnly => {
            "transport error (insecure request requires HTTPS)"
        }
        ureq::ErrorKind::ConnectionFailed => "transport error (connection failed)",
        ureq::ErrorKind::TooManyRedirects => "transport error (too many redirects)",
        ureq::ErrorKind::BadStatus => "transport error (invalid response status line)",
        ureq::ErrorKind::BadHeader => "transport error (invalid header)",
        ureq::ErrorKind::Io => "transport error (I/O error)",
        ureq::ErrorKind::InvalidProxyUrl => "transport error (malformed proxy URL)",
        ureq::ErrorKind::ProxyConnect => "transport error (proxy connection failed)",
        ureq::ErrorKind::ProxyUnauthorized => "transport error (proxy credentials rejected)",
        // `ErrorKind::HTTP` describes a status response, so it should not reach
        // this path; the arm exists because the enum is exhaustive.
        ureq::ErrorKind::HTTP => "transport error (unexpected HTTP status)",
    }
}
