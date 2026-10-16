//! What an HTTP response means for the retry loop.
//!
//! A classification rather than a policy: this module says which responses are
//! worth another attempt, and the worker decides what an attempt costs and how
//! long it may go on for. Keeping the two apart is what lets the retry deadline
//! and the shutdown deadline both bound the worker - they read the same verdict
//! and disagree only about whether there is still time.

/// Classification of HTTP response for retry logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseClass {
    /// 2xx responses - request succeeded.
    Success,
    /// 5xx, 429, or network errors - retry with backoff.
    Retryable,
    /// 4xx (except 429) - permanent failure, do not retry.
    Permanent,
}

/// Classifies an HTTP status code for retry logic.
///
/// # Classification rules
///
/// * **2xx** → [`ResponseClass::Success`] - request completed successfully
/// * **429** → [`ResponseClass::Retryable`] - rate limited, retry with backoff
/// * **5xx** → [`ResponseClass::Retryable`] - server error, retry with backoff
/// * **Other** → [`ResponseClass::Permanent`] - client error (4xx except 429), do not retry
///
/// # Examples
///
/// ```ignore
/// assert_eq!(classify_status(200), ResponseClass::Success);
/// assert_eq!(classify_status(429), ResponseClass::Retryable);
/// assert_eq!(classify_status(404), ResponseClass::Permanent);
/// ```
pub(crate) fn classify_status(status: u16) -> ResponseClass {
    match status {
        200..=299 => ResponseClass::Success,
        429 => ResponseClass::Retryable,
        500..=599 => ResponseClass::Retryable,
        _ => ResponseClass::Permanent,
    }
}
