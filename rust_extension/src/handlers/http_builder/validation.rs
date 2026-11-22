//! Endpoint validation for the HTTP handler builder.
//!
//! The check that an endpoint can actually be sent is separate from the rest of
//! the builder's value checks because it has to agree with the transport's own
//! parser. Keeping it beside the builder, rather than inside the transport,
//! means an endpoint is reported once, when the handler is built, instead of on
//! every attempt.

use ureq::http::Uri;
use url::Url;

use super::{HTTPHandlerBuilder, HandlerBuildError};

/// The error reported when a builder has no endpoint configured.
pub(super) const MISSING_ENDPOINT: &str = "HTTP handler requires a URL";

impl HTTPHandlerBuilder {
    /// Reject an endpoint the transport could never send.
    ///
    /// ureq 2 parsed every request URL with `url::Url`; ureq 3 parses the same
    /// string as an `http::Uri` when it sends, and rejects what the first
    /// parser accepts. An endpoint either parser refuses, or one whose scheme
    /// the transport cannot send, would leave the worker retrying every record
    /// until the backoff deadline, so it is reported once, when the handler is
    /// built, rather than on every attempt. The endpoint itself is kept as
    /// configured: the transport parses each request target exactly as ureq 2
    /// did, which is also what keeps a space in a path percent-encoded.
    pub(super) fn validate_endpoint(&self) -> Result<(), HandlerBuildError> {
        let Some(url) = self.url.as_deref() else {
            return Err(HandlerBuildError::InvalidConfig(MISSING_ENDPOINT.into()));
        };
        if url.trim().is_empty() {
            return Err(HandlerBuildError::InvalidConfig(
                "URL must not be empty".into(),
            ));
        }
        let parsed = Url::parse(url).map_err(invalid_endpoint)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(HandlerBuildError::InvalidConfig(
                "invalid HTTP endpoint: the scheme must be http or https".into(),
            ));
        }
        // The send path parses the normalized form, so an endpoint it rejects
        // must be rejected here too.
        parsed
            .to_string()
            .parse::<Uri>()
            .map(|_| ())
            .map_err(invalid_endpoint)
    }
}

/// Report an endpoint the transport cannot send.
///
/// The parser's own message names the defect without echoing the endpoint,
/// which may carry userinfo.
fn invalid_endpoint(err: impl std::fmt::Display) -> HandlerBuildError {
    HandlerBuildError::InvalidConfig(format!("invalid HTTP endpoint: {err}"))
}
