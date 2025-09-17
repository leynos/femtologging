//! Overflow policy parsing helpers for the file handler.
//!
//! Provides shared parsing logic for Python-facing APIs so behaviour
//! remains consistent between constructors and builder methods.

use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::config::OverflowPolicy;

const VALID_POLICIES: &str = "drop, block, timeout:N";

fn parse_overflow_policy(
    policy: &str,
    timeout_ms: Option<u64>,
    expects_external_timeout: bool,
) -> PyResult<OverflowPolicy> {
    let trimmed = policy.trim();
    let normalized = trimmed.to_ascii_lowercase();

    if let Some(rest) = normalized.strip_prefix("timeout:") {
        let ms: i64 = rest.trim().parse().map_err(|_| {
            PyValueError::new_err("timeout must be a positive integer (N in 'timeout:N')")
        })?;
        if ms <= 0 {
            return Err(PyValueError::new_err("timeout must be greater than zero"));
        }
        return Ok(OverflowPolicy::Timeout(Duration::from_millis(ms as u64)));
    }

    match normalized.as_str() {
        "drop" => Ok(OverflowPolicy::Drop),
        "block" => Ok(OverflowPolicy::Block),
        "timeout" => {
            if !expects_external_timeout {
                return Err(PyValueError::new_err(
                    "timeout requires a positive integer N, use 'timeout:N'",
                ));
            }

            let ms = timeout_ms
                .ok_or_else(|| PyValueError::new_err("timeout_ms required for timeout policy"))?;

            if ms == 0 {
                return Err(PyValueError::new_err(
                    "timeout_ms must be greater than zero",
                ));
            }

            Ok(OverflowPolicy::Timeout(Duration::from_millis(ms)))
        }
        _ => Err(PyValueError::new_err(format!(
            "invalid overflow policy '{normalized}'. Valid options are: {VALID_POLICIES}"
        ))),
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
///     policy::parse_policy_string("drop").unwrap(),
///     OverflowPolicy::Drop
/// ));
/// assert!(matches!(
///     policy::parse_policy_string("timeout:1000").unwrap(),
///     OverflowPolicy::Timeout(_)
/// ));
/// ```
pub(crate) fn parse_policy_string(policy: &str) -> PyResult<OverflowPolicy> {
    parse_overflow_policy(policy, None, false)
}

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
        assert_eq!(parse_policy_string(" drop ").unwrap(), OverflowPolicy::Drop);
    }

    #[test]
    fn parse_policy_string_accepts_block_case_insensitive() {
        assert_eq!(parse_policy_string("BLOCK").unwrap(), OverflowPolicy::Block);
    }

    #[test]
    fn parse_policy_string_parses_timeout_values() {
        assert_eq!(
            parse_policy_string("timeout:250").unwrap(),
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
                parse_policy_string("timeout:abc").unwrap_err(),
                "timeout must be a positive integer (N in 'timeout:N')",
            );
        }

        #[test]
        fn parse_policy_string_rejects_zero_timeout_value() {
            assert_value_error_message(
                parse_policy_string("timeout:0").unwrap_err(),
                "timeout must be greater than zero",
            );
        }

        #[test]
        fn parse_policy_string_reports_missing_timeout_hint() {
            assert_value_error_message(
                parse_policy_string("timeout").unwrap_err(),
                "timeout requires a positive integer N, use 'timeout:N'",
            );
        }

        #[test]
        fn parse_policy_string_rejects_unknown_policy_message() {
            assert_value_error_message(
                parse_policy_string("unknown").unwrap_err(),
                "invalid overflow policy 'unknown'. Valid options are: drop, block, timeout:N",
            );
        }

        #[test]
        fn parse_policy_with_timeout_supports_drop() {
            assert_eq!(
                parse_policy_with_timeout("drop", None).unwrap(),
                OverflowPolicy::Drop
            );
        }

        #[test]
        fn parse_policy_with_timeout_requires_timeout_value() {
            assert_value_error_message(
                parse_policy_with_timeout("timeout", None).unwrap_err(),
                "timeout_ms required for timeout policy",
            );
        }

        #[test]
        fn parse_policy_with_timeout_rejects_zero_timeout_value() {
            assert_value_error_message(
                parse_policy_with_timeout("timeout", Some(0)).unwrap_err(),
                "timeout_ms must be greater than zero",
            );
        }

        #[test]
        fn parse_policy_with_timeout_accepts_timeout_value() {
            assert_eq!(
                parse_policy_with_timeout("timeout", Some(500)).unwrap(),
                OverflowPolicy::Timeout(Duration::from_millis(500))
            );
        }

        #[test]
        fn parse_policy_with_timeout_handles_inline_value() {
            assert_eq!(
                parse_policy_with_timeout("timeout:125", None).unwrap(),
                OverflowPolicy::Timeout(Duration::from_millis(125))
            );
        }
    }
}
