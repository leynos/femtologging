//! Unit tests for the redirect rules ureq 2 applied.
//!
//! The loopback tests in `request_shape_tests/redirects.rs` prove the rules on
//! the wire. What lives here is the decision itself, including the statuses and
//! header shapes a loopback fixture cannot easily produce.

use std::sync::OnceLock;

use rstest::rstest;
use ureq::http::HeaderValue;
use url::Url;

use super::*;

/// The base URL a redirect is resolved against, shared so attempts can borrow
/// it for the lifetime of a test.
fn base() -> &'static Url {
    static BASE: OnceLock<Url> = OnceLock::new();
    BASE.get_or_init(|| Url::parse("http://collector.example/log").expect("the base parses"))
}

/// A `Location` header carrying `value`.
fn location(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("the test value is a header value")
}

/// One response in a chain rooted at [`base`], with `method` and `status`.
fn attempt<'a>(
    method: &'a HTTPMethod,
    status: u16,
    location: Option<&'a HeaderValue>,
    responses_seen: usize,
) -> RedirectAttempt<'a> {
    RedirectAttempt {
        base: base(),
        method,
        status,
        location,
        responses_seen,
    }
}

/// Scenario: a redirect response that carries a usable `Location`.
///
/// Invariant: the next request uses the method ureq 2 replayed, and its target
/// is the `Location` resolved against the previous URL.
#[rstest]
#[case::moved_permanently(301, HTTPMethod::POST, "http://other.example/a")]
#[case::found(302, HTTPMethod::POST, "http://other.example/a")]
#[case::see_other(303, HTTPMethod::POST, "http://other.example/a")]
#[case::temporary_redirect_for_a_get(307, HTTPMethod::GET, "http://other.example/a")]
#[case::permanent_redirect_for_a_get(308, HTTPMethod::GET, "http://other.example/a")]
#[case::relative_location(302, HTTPMethod::POST, "/alternate")]
fn a_replayable_redirect_is_followed(
    #[case] status: u16,
    #[case] method: HTTPMethod,
    #[case] target: &str,
) {
    let hop = next_hop(attempt(&method, status, Some(&location(target)), 1));

    let NextHop::Follow { url, method } = hop else {
        panic!("a replayable redirect must be followed: {hop:?}");
    };
    assert_eq!(method, HTTPMethod::GET);
    let expected = base().join(target).expect("the target resolves");
    assert_eq!(url, expected);
}

/// Scenario: a 3xx that ureq 2 returned to the caller instead of following.
///
/// Invariant: the status is left for the classifier, as it was under ureq 2,
/// which broke out of its loop. A 307 or 308 answering a POST would replay the
/// request body, and 300, 304 and 305 have no replay rule at all.
#[rstest]
#[case::multiple_choices(300)]
#[case::not_modified(304)]
#[case::use_proxy(305)]
#[case::temporary_redirect_of_a_post(307)]
#[case::permanent_redirect_of_a_post(308)]
fn a_redirect_ureq_two_would_not_replay_is_refused(#[case] status: u16) {
    let hop = next_hop(attempt(
        &HTTPMethod::POST,
        status,
        Some(&location("http://other.example/a")),
        1,
    ));

    assert!(matches!(hop, NextHop::Refuse), "{hop:?}");
}

/// Scenario: a redirect with no usable `Location`.
///
/// Invariant: a missing header and one that is not valid UTF-8 both leave the
/// response to be classified, which is what ureq 2 did when its header lookup
/// returned `None`.
#[rstest]
#[case::missing(None)]
#[case::not_utf8(Some(HeaderValue::from_bytes(b"http://other.example/\x80").expect("opaque bytes are a header value")))]
fn a_redirect_without_a_location_is_refused(#[case] location: Option<HeaderValue>) {
    let hop = next_hop(attempt(&HTTPMethod::POST, 302, location.as_ref(), 1));

    assert!(matches!(hop, NextHop::Refuse), "{hop:?}");
}

/// Scenario: the chain has already seen ureq 2's whole allowance.
///
/// Invariant: the attempt fails as a transport error, even when the redirect
/// itself would otherwise be followable and carries no `Location`, because
/// ureq 2 checked its limit before it looked at the header.
#[rstest]
#[case::with_a_location(Some(location("http://other.example/a")))]
#[case::without_a_location(None)]
fn the_chain_limit_is_checked_first(#[case] location: Option<HeaderValue>) {
    let hop = next_hop(attempt(
        &HTTPMethod::GET,
        302,
        location.as_ref(),
        MAX_RESPONSES,
    ));

    assert!(
        matches!(hop, NextHop::Failed("reached max redirects")),
        "{hop:?}"
    );
}

/// Scenario: a `Location` value that cannot be resolved against the base.
///
/// Invariant: the attempt fails with the fixed `invalid redirect location`
/// label, and the server-supplied value is never part of that diagnostic.
#[test]
fn an_unresolvable_location_is_reported_without_its_value() {
    let bad = location("http://[::1");
    let hop = next_hop(attempt(&HTTPMethod::GET, 302, Some(&bad), 1));

    assert!(
        matches!(hop, NextHop::Failed("invalid redirect location")),
        "{hop:?}"
    );
}
