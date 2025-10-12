//! Overflow policy parsing helpers for the file handler.
//!
//! Provides shared parsing logic for Python-facing APIs so behaviour
//! remains consistent between constructors and builder methods.

use std::time::Duration;

use super::config::OverflowPolicy;
use thiserror::Error;

const VALID_POLICIES: &str = "drop, block, timeout:N";

/// Errors produced while parsing overflow policy inputs.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ParseOverflowPolicyError {
    /// Timeout values must be positive integers.
    #[error("timeout must be a positive integer (N in 'timeout:N')")]
    InvalidTimeoutValue,
    /// Timeout values may not be zero.
    #[error("timeout must be greater than zero")]
    TimeoutZero,
    /// Inline timeout syntax is required when an external timeout is not expected.
    #[error("timeout requires a positive integer N, use 'timeout:N'")]
    InlineTimeoutRequired,
    /// A timeout policy without inline value requires an external timeout.
    #[error("timeout_ms required for timeout policy")]
    MissingExternalTimeout,
    /// Provided policy name is not recognised.
    #[error("invalid overflow policy: '{policy}'. Valid options are: {VALID_POLICIES}")]
    UnknownPolicy { policy: String },
}

fn parse_timeout_ms(raw: &str) -> Result<u64, ParseOverflowPolicyError> {
    let ms: u64 = raw
        .trim()
        .parse()
        .map_err(|_| ParseOverflowPolicyError::InvalidTimeoutValue)?;
    if ms == 0 {
        Err(ParseOverflowPolicyError::TimeoutZero)
    } else {
        Ok(ms)
    }
}

fn parse_timeout_policy(
    timeout_ms: Option<u64>,
    expects_external_timeout: bool,
) -> Result<OverflowPolicy, ParseOverflowPolicyError> {
    if !expects_external_timeout {
        return Err(ParseOverflowPolicyError::InlineTimeoutRequired);
    }

    let ms = timeout_ms.ok_or(ParseOverflowPolicyError::MissingExternalTimeout)?;

    if ms == 0 {
        Err(ParseOverflowPolicyError::TimeoutZero)
    } else {
        Ok(OverflowPolicy::Timeout(Duration::from_millis(ms)))
    }
}

fn parse_overflow_policy(
    policy: &str,
    timeout_ms: Option<u64>,
    expects_external_timeout: bool,
) -> Result<OverflowPolicy, ParseOverflowPolicyError> {
    let normalized = policy.trim().to_ascii_lowercase();
    // Split inline timeout once, then dispatch via a single match.
    let (kind, inline_ms) = if let Some(rest) = normalized.strip_prefix("timeout:") {
        ("timeout", Some(parse_timeout_ms(rest)?))
    } else {
        (normalized.as_str(), None)
    };

    match (kind, inline_ms, expects_external_timeout) {
        ("drop", _, _) => Ok(OverflowPolicy::Drop),
        ("block", _, _) => Ok(OverflowPolicy::Block),
        ("timeout", Some(ms), _) => Ok(OverflowPolicy::Timeout(Duration::from_millis(ms))),
        ("timeout", None, _) => parse_timeout_policy(timeout_ms, expects_external_timeout),
        (_, _, _) => Err(ParseOverflowPolicyError::UnknownPolicy { policy: normalized }),
    }
}

/// Parses a policy string into an [`OverflowPolicy`].
///
/// # Accepted input formats
/// - "drop": Drop new items when the buffer is full.
/// - "block": Block until space is available.
/// - "timeout:N": Wait up to N milliseconds before dropping (N is a positive integer).
///
/// # Errors
/// Returns a [`ParseOverflowPolicyError`] if the input string is not a valid policy.
///
/// # Examples
/// ```ignore
/// # use femtologging_rs::handlers::file::policy;
/// # use femtologging_rs::handlers::file::OverflowPolicy;
/// assert!(matches!(
///     policy::parse_policy_string("drop").expect("parse drop"),
///     OverflowPolicy::Drop
/// ));
/// assert!(matches!(
///     policy::parse_policy_string("timeout:1000").expect("parse timeout:1000"),
///     OverflowPolicy::Timeout(_)
/// ));
/// ```
pub(crate) fn parse_policy_string(
    policy: &str,
) -> Result<OverflowPolicy, ParseOverflowPolicyError> {
    parse_overflow_policy(policy, None, false)
}

/// Parses a policy string with an optional external timeout.
///
/// Precedence: if `policy` contains `timeout:N`, that inline value is used and
/// any provided `timeout_ms` is ignored.
#[cfg(feature = "python")]
pub(crate) fn parse_policy_with_timeout(
    policy: &str,
    timeout_ms: Option<u64>,
) -> Result<OverflowPolicy, ParseOverflowPolicyError> {
    parse_overflow_policy(policy, timeout_ms, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_policy_string_accepts_drop_whitespace() {
        assert_eq!(
            parse_policy_string(" drop ").expect("parse drop"),
            OverflowPolicy::Drop
        );
    }

    #[test]
    fn parse_policy_string_accepts_block_case_insensitive() {
        assert_eq!(
            parse_policy_string("BLOCK").expect("parse block"),
            OverflowPolicy::Block
        );
    }

    #[test]
    fn parse_policy_string_parses_timeout_values() {
        assert_eq!(
            parse_policy_string("timeout:250").expect("parse timeout:250"),
            OverflowPolicy::Timeout(Duration::from_millis(250))
        );
    }

    #[test]
    fn parse_policy_string_rejects_missing_timeout_value() {
        assert!(parse_policy_string("timeout").is_err());
    }

    #[test]
    fn parse_policy_string_rejects_unknown_policy() {
        assert!(parse_policy_string("unknown").is_err());
    }

    #[test]
    fn parse_policy_string_rejects_non_numeric_timeout_value() {
        let err =
            parse_policy_string("timeout:abc").expect_err("expect parse error for timeout:abc");
        assert_eq!(err, ParseOverflowPolicyError::InvalidTimeoutValue);
        assert_eq!(
            err.to_string(),
            "timeout must be a positive integer (N in 'timeout:N')"
        );
    }

    #[test]
    fn parse_policy_string_rejects_zero_timeout_value() {
        let err =
            parse_policy_string("timeout:0").expect_err("expect parse error for zero timeout");
        assert_eq!(err, ParseOverflowPolicyError::TimeoutZero);
        assert_eq!(err.to_string(), "timeout must be greater than zero");
    }

    #[test]
    fn parse_policy_string_reports_missing_timeout_hint() {
        let err =
            parse_policy_string("timeout").expect_err("expect parse error for missing timeout");
        assert_eq!(err, ParseOverflowPolicyError::InlineTimeoutRequired);
        assert_eq!(
            err.to_string(),
            "timeout requires a positive integer N, use 'timeout:N'"
        );
    }

    #[test]
    fn parse_policy_string_rejects_unknown_policy_message() {
        let err =
            parse_policy_string("unknown").expect_err("expect parse error for unknown policy");
        assert_eq!(
            err,
            ParseOverflowPolicyError::UnknownPolicy {
                policy: "unknown".into(),
            }
        );
        assert_eq!(
            err.to_string(),
            "invalid overflow policy: 'unknown'. Valid options are: drop, block, timeout:N"
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn parse_policy_with_timeout_supports_drop() {
        assert_eq!(
            parse_policy_with_timeout("drop", None).expect("parse drop with external"),
            OverflowPolicy::Drop
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn parse_policy_with_timeout_requires_timeout_value() {
        let err = parse_policy_with_timeout("timeout", None)
            .expect_err("expect missing external timeout");
        assert_eq!(err, ParseOverflowPolicyError::MissingExternalTimeout);
        assert_eq!(err.to_string(), "timeout_ms required for timeout policy");
    }

    #[cfg(feature = "python")]
    #[test]
    fn parse_policy_with_timeout_rejects_zero_timeout_value() {
        let err = parse_policy_with_timeout("timeout", Some(0))
            .expect_err("expect zero external timeout error");
        assert_eq!(err, ParseOverflowPolicyError::TimeoutZero);
        assert_eq!(err.to_string(), "timeout must be greater than zero");
    }

    #[cfg(feature = "python")]
    #[test]
    fn parse_policy_with_timeout_accepts_timeout_value() {
        assert_eq!(
            parse_policy_with_timeout("timeout", Some(500)).expect("parse timeout with external"),
            OverflowPolicy::Timeout(Duration::from_millis(500))
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn parse_policy_with_timeout_handles_inline_value() {
        assert_eq!(
            parse_policy_with_timeout("timeout:125", None)
                .expect("parse inline timeout with external path"),
            OverflowPolicy::Timeout(Duration::from_millis(125))
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn parse_policy_with_timeout_prefers_inline_timeout() {
        assert_eq!(
            parse_policy_with_timeout("timeout:125", Some(500))
                .expect("inline timeout should take precedence"),
            OverflowPolicy::Timeout(Duration::from_millis(125))
        );
    }
}
