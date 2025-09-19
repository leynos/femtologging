//! Overflow policy parsing helpers for the file handler.
//!
//! Provides shared parsing logic for Python-facing APIs so behaviour
//! remains consistent between constructors and builder methods.

use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::config::OverflowPolicy;

const VALID_POLICIES: &str = "drop, block, timeout:N";

fn timeout_parse_error() -> PyErr {
    PyValueError::new_err("timeout must be a positive integer (N in 'timeout:N')")
}

fn timeout_zero_error() -> PyErr {
    PyValueError::new_err("timeout must be greater than zero")
}

fn invalid_policy_error(policy: &str) -> PyErr {
    PyValueError::new_err(format!(
        "invalid overflow policy '{policy}'. Valid options are: {VALID_POLICIES}"
    ))
}

fn ensure_timeout_positive(ms: u64) -> PyResult<u64> {
    if ms == 0 {
        Err(timeout_zero_error())
    } else {
        Ok(ms)
    }
}

fn parse_timeout_ms(raw: &str) -> PyResult<u64> {
    let ms: u64 = raw.trim().parse().map_err(|_| timeout_parse_error())?;
    ensure_timeout_positive(ms)
}

fn parse_timeout_policy(
    timeout_ms: Option<u64>,
    expects_external_timeout: bool,
) -> PyResult<OverflowPolicy> {
    match (expects_external_timeout, timeout_ms) {
        (false, _) => Err(PyValueError::new_err(
            "timeout requires a positive integer N, use 'timeout:N'",
        )),
        (true, Some(ms)) => ensure_timeout_positive(ms)
            .map(|value| OverflowPolicy::Timeout(Duration::from_millis(value))),
        (true, None) => Err(PyValueError::new_err(
            "timeout_ms required for timeout policy",
        )),
    }
}

fn parse_overflow_policy(
    policy: &str,
    timeout_ms: Option<u64>,
    expects_external_timeout: bool,
) -> PyResult<OverflowPolicy> {
    let trimmed = policy.trim();
    let normalized = trimmed.to_ascii_lowercase();

    if let Some(rest) = normalized.strip_prefix("timeout:") {
        return parse_timeout_ms(rest).map(|ms| OverflowPolicy::Timeout(Duration::from_millis(ms)));
    }

    match normalized.as_str() {
        "drop" => Ok(OverflowPolicy::Drop),
        "block" => Ok(OverflowPolicy::Block),
        "timeout" => parse_timeout_policy(timeout_ms, expects_external_timeout),
        _ => Err(invalid_policy_error(&normalized)),
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
/// Returns a [`PyValueError`] if the input string is not a valid policy.
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
pub(crate) fn parse_policy_string(policy: &str) -> PyResult<OverflowPolicy> {
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
) -> PyResult<OverflowPolicy> {
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

    #[cfg(feature = "python")]
    mod python {
        use super::*;
        use pyo3::Python;

        fn assert_value_error_message(err: PyErr, expected: &str) {
            Python::with_gil(|py| {
                assert!(err.is_instance_of::<PyValueError>(py));
                assert_eq!(err.value(py).to_string(), expected);
            });
        }

        #[test]
        fn parse_policy_string_rejects_non_numeric_timeout_value() {
            assert_value_error_message(
                parse_policy_string("timeout:abc").expect_err("expect parse error for timeout:abc"),
                "timeout must be a positive integer (N in 'timeout:N')",
            );
        }

        #[test]
        fn parse_policy_string_rejects_zero_timeout_value() {
            assert_value_error_message(
                parse_policy_string("timeout:0").expect_err("expect parse error for zero timeout"),
                "timeout must be greater than zero",
            );
        }

        #[test]
        fn parse_policy_string_reports_missing_timeout_hint() {
            assert_value_error_message(
                parse_policy_string("timeout").expect_err("expect parse error for missing timeout"),
                "timeout requires a positive integer N, use 'timeout:N'",
            );
        }

        #[test]
        fn parse_policy_string_rejects_unknown_policy_message() {
            assert_value_error_message(
                parse_policy_string("unknown").expect_err("expect parse error for unknown policy"),
                "invalid overflow policy 'unknown'. Valid options are: drop, block, timeout:N",
            );
        }

        #[test]
        fn parse_policy_with_timeout_supports_drop() {
            assert_eq!(
                parse_policy_with_timeout("drop", None).expect("parse drop with external"),
                OverflowPolicy::Drop
            );
        }

        #[test]
        fn parse_policy_with_timeout_requires_timeout_value() {
            assert_value_error_message(
                parse_policy_with_timeout("timeout", None)
                    .expect_err("expect missing external timeout"),
                "timeout_ms required for timeout policy",
            );
        }

        #[test]
        fn parse_policy_with_timeout_rejects_zero_timeout_value() {
            assert_value_error_message(
                parse_policy_with_timeout("timeout", Some(0))
                    .expect_err("expect zero external timeout error"),
                "timeout must be greater than zero",
            );
        }

        #[test]
        fn parse_policy_with_timeout_accepts_timeout_value() {
            assert_eq!(
                parse_policy_with_timeout("timeout", Some(500))
                    .expect("parse timeout with external"),
                OverflowPolicy::Timeout(Duration::from_millis(500))
            );
        }

        #[test]
        fn parse_policy_with_timeout_handles_inline_value() {
            assert_eq!(
                parse_policy_with_timeout("timeout:125", None)
                    .expect("parse inline timeout with external path"),
                OverflowPolicy::Timeout(Duration::from_millis(125))
            );
        }

        #[test]
        fn parse_policy_with_timeout_prefers_inline_timeout() {
            assert_eq!(
                parse_policy_with_timeout("timeout:125", Some(500))
                    .expect("inline timeout should take precedence"),
                OverflowPolicy::Timeout(Duration::from_millis(125))
            );
        }
    }
}
