//! Application-owned descriptions of transport failures.
//!
//! `ureq::Error`'s `Display` output is never logged. Its `BadUri` variant
//! repeats the failed URI, which can carry the configured URL's userinfo and
//! query values, and the other variants wrap dependency errors whose text is
//! not part of this application's diagnostic contract. Classifying the error
//! keeps the warning useful without echoing caller-supplied configuration.

use ureq::Error;

/// Describe a transport failure without exposing dependency error text.
///
/// Every arm returns a fixed string literal, and the retry path accepts only
/// `&'static str`, so borrowing a dependency's runtime text into the warning
/// fails to compile. The type alone does not prove provenance: `Box::leak`
/// can manufacture a static reference from runtime text. The guarantee rests
/// on these arms returning literals. `Error` is `#[non_exhaustive]`, so the
/// final arm covers a variant a later `ureq` release may add.
///
/// The variants gated behind optional `ureq` features are covered by that
/// wildcard rather than named here, so this module compiles under every
/// feature selection the dependency graph produces.
pub(super) fn classification(error: &Error) -> &'static str {
    match error {
        // `execute` classifies a status before calling this function; the arm
        // exists so that a caller cannot turn a verdict into a failure label.
        Error::StatusCode(_) => "transport error (unexpected HTTP status)",
        Error::Http(_) => "transport error (invalid request)",
        Error::BadUri(_) => "transport error (invalid URL)",
        Error::Protocol(_) => "transport error (protocol error)",
        Error::Io(_) => "transport error (I/O error)",
        Error::Timeout(_) => "transport error (timed out)",
        Error::HostNotFound => "transport error (DNS lookup failed)",
        Error::RedirectFailed => "transport error (redirect refused)",
        Error::InvalidProxyUrl => "transport error (malformed proxy URL)",
        Error::ConnectionFailed => "transport error (connection failed)",
        Error::BodyExceedsLimit(_) => "transport error (request body too large)",
        Error::TooManyRedirects => "transport error (too many redirects)",
        Error::Tls(_) => "transport error (TLS failure)",
        Error::RequireHttpsOnly(_) => "transport error (insecure request requires HTTPS)",
        Error::LargeResponseHeader(_, _) => "transport error (response header too large)",
        Error::ConnectProxyFailed(_) => "transport error (proxy connection failed)",
        Error::TlsRequired => "transport error (TLS required)",
        Error::Other(_) => "transport error (unrecognised failure)",
        // `ureq` documents this as a state it should never produce, but the
        // variant is public and unconditional, so it is named rather than left
        // to the wildcard.
        Error::BodyStalled => "transport error (response body stalled)",
        _ => "transport error (unrecognised failure)",
    }
}

#[cfg(test)]
mod tests {
    //! Regression tests for the credential-free failure vocabulary.

    use super::*;

    /// Build a list of `(error, label)` rows, one row per source line.
    macro_rules! variant_rows {
        ($($error:expr => $label:literal),+ $(,)?) => {
            vec![$(($error, $label)),+]
        };
    }

    /// An `http::Error` from a header value the client cannot send.
    fn invalid_header_error() -> ureq::http::Error {
        ureq::http::HeaderValue::from_bytes(b"\n")
            .expect_err("a control byte is not a header value")
            .into()
    }

    /// Every unconditional `ureq::Error` variant, with the label it must map to.
    ///
    /// The variants gated behind optional `ureq` features are absent because
    /// the dependency graph does not enable them; they share the wildcard arm.
    fn unconditional_variants() -> Vec<(Error, &'static str)> {
        variant_rows![
            Error::StatusCode(503) => "transport error (unexpected HTTP status)",
            Error::Http(invalid_header_error()) => "transport error (invalid request)",
            Error::BadUri("http://example.test/".into()) => "transport error (invalid URL)",
            Error::Io(std::io::Error::other("connection reset")) => "transport error (I/O error)",
            Error::Timeout(ureq::Timeout::Global) => "transport error (timed out)",
            Error::HostNotFound => "transport error (DNS lookup failed)",
            Error::RedirectFailed => "transport error (redirect refused)",
            Error::InvalidProxyUrl => "transport error (malformed proxy URL)",
            Error::ConnectionFailed => "transport error (connection failed)",
            Error::BodyExceedsLimit(1024) => "transport error (request body too large)",
            Error::TooManyRedirects => "transport error (too many redirects)",
            Error::Tls("handshake failed") => "transport error (TLS failure)",
            Error::RequireHttpsOnly("http://example.test/".into())
                => "transport error (insecure request requires HTTPS)",
            Error::LargeResponseHeader(64 * 1024, 8 * 1024)
                => "transport error (response header too large)",
            Error::ConnectProxyFailed("proxy refused".into())
                => "transport error (proxy connection failed)",
            Error::TlsRequired => "transport error (TLS required)",
            Error::Other("bespoke connector failure".into())
                => "transport error (unrecognised failure)",
            Error::BodyStalled => "transport error (response body stalled)",
        ]
    }

    /// Scenario: each unconditional `ureq::Error` variant is classified.
    ///
    /// Invariant: every variant maps to its own exact label, so swapping two
    /// arms or collapsing them to one generic label fails here. The protocol
    /// variant is absent because `ureq` does not re-export `ureq_proto::Error`,
    /// which leaves no public constructor for it; its arm returns a literal.
    #[test]
    fn every_constructible_variant_has_its_own_label() {
        for (error, expected) in unconditional_variants() {
            assert_eq!(classification(&error), expected, "for {error:?}");
        }
    }

    /// Scenario: the labels are distinct and free of any configured value.
    ///
    /// Invariant: a shared label would make the fixed vocabulary useless for
    /// diagnosis, and no label may repeat the URI or message it replaces.
    #[test]
    fn the_labels_are_distinct_and_carry_no_payload() {
        let labels: Vec<&'static str> = unconditional_variants()
            .iter()
            .map(|(error, _)| classification(error))
            .collect();
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            labels.len(),
            "labels must be distinct: {labels:?}"
        );

        for label in labels {
            for secret in [
                "synthetic-user",
                "SYNTHETIC_PASSWORD",
                "SYNTHETIC_QUERY_SECRET",
                "example.test",
            ] {
                assert!(
                    !label.contains(secret),
                    "classification leaked {secret}: {label:?}"
                );
            }
        }
    }

    /// Scenario: the dependency's error carries the failed URI, including a
    /// synthetic userinfo password and query value.
    ///
    /// Invariant: the classification is application-owned text, so none of
    /// those values can reach the warning the worker logs.
    #[test]
    fn a_uri_is_never_part_of_the_classification() {
        let error = Error::BadUri(
            "http://synthetic-user:SYNTHETIC_PASSWORD@127.0.0.1/ingest\
             ?api_key=SYNTHETIC_QUERY_SECRET"
                .to_owned(),
        );

        let description = classification(&error);

        assert_eq!(description, "transport error (invalid URL)");
        for secret in [
            "synthetic-user",
            "SYNTHETIC_PASSWORD",
            "SYNTHETIC_QUERY_SECRET",
        ] {
            assert!(
                !description.contains(secret),
                "classification leaked {secret}: {description:?}"
            );
        }
    }
}
