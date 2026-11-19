//! Redirect following with the rules ureq 2 applied.
//!
//! ureq 3 follows every 3xx that carries a usable `Location`, resolves it as an
//! `http::Uri`, and reports a redirect it will not replay as an error. ureq 2
//! followed only the statuses whose method it could replay safely, resolved the
//! header with `url::Url`, read a header it could not decode as absent, and
//! returned every other 3xx to the caller, which classified its status.
//!
//! Both differences are observable. A `300 Multiple Choices` that names an
//! alternative representation would be requested a second time as a GET, and a
//! `200` there would be recorded as a delivered log record even though the
//! collector never accepted the original request. A redirect whose `Location`
//! holds an unencoded space reached its target under ureq 2 and would fail
//! under ureq 3. Following the chain here, with the agent's own following
//! disabled, keeps the contract in one place that the loopback tests assert.

use ureq::http::HeaderValue;
use url::Url;

use super::config::HTTPMethod;

/// Responses ureq 2 allowed in a redirect chain, counting the original request.
///
/// ureq 2 was configured with five and counted the original request against
/// them, so it followed four redirects and failed on the fifth redirect
/// response. ureq 3 counts only the redirects it follows.
pub(super) const MAX_RESPONSES: usize = 5;

/// What the transport should do with a 3xx response.
#[derive(Debug)]
pub(super) enum NextHop {
    /// Follow the redirect by requesting `url` with `method`.
    Follow {
        /// Absolute target, joined to the URL of the request that redirected.
        url: Url,
        /// Method ureq 2 used for this status.
        method: HTTPMethod,
    },
    /// Return the response, whose status the caller classifies.
    Refuse,
    /// End the attempt as a transport error, as ureq 2 did.
    Failed(String),
}

/// Decide what a 3xx response means for the request that produced it.
///
/// The checks run in ureq 2's order: the chain limit first, then whether a
/// decodable `Location` exists, then whether it resolves against the previous
/// URL, and only then whether the status and method may be replayed. A header
/// that is not valid UTF-8 counts as absent, which is what ureq 2's
/// `header("location")` returned.
pub(super) fn next_hop(
    base: &Url,
    method: &HTTPMethod,
    status: u16,
    location: Option<&HeaderValue>,
    responses_seen: usize,
) -> NextHop {
    if responses_seen >= MAX_RESPONSES {
        return NextHop::Failed(format!("reached max redirects ({MAX_RESPONSES})"));
    }
    let Some(location) = location.and_then(|value| value.to_str().ok()) else {
        return NextHop::Refuse;
    };
    let url = match base.join(location) {
        Ok(url) => url,
        Err(err) => return NextHop::Failed(format!("invalid redirect location: {err}")),
    };
    match redirected_method(method, status) {
        Some(method) => NextHop::Follow { url, method },
        None => NextHop::Refuse,
    }
}

/// The method ureq 2 replayed a redirect with, or `None` when it refused.
///
/// ureq 2 rewrote 301, 302, and 303 to `GET`, kept a method that cannot carry a
/// body for 307 and 308, and returned every other 3xx to the caller: 300, 304
/// and 305 have no replay rule, and a 307 or 308 that would resend a body is
/// refused.
fn redirected_method(method: &HTTPMethod, status: u16) -> Option<HTTPMethod> {
    match status {
        301..=303 => Some(HTTPMethod::GET),
        307 | 308 if *method == HTTPMethod::GET => Some(HTTPMethod::GET),
        _ => None,
    }
}

#[cfg(test)]
#[path = "redirect_tests.rs"]
mod tests;
