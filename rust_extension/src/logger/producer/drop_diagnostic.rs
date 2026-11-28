//! The diagnostic emitted for a record dropped over an invalid context payload.
//!
//! The producer counts the dropped record and writes the reason to a caller
//! supplied writer. Keeping the wording and the write separate lets a test pin
//! the exact text, while the prebuilt-record path proves that production
//! reaches the write without redirecting the process's stderr descriptor.

use crate::log_context;

/// The diagnostic a record dropped for an invalid context writes to stderr.
///
/// The write itself targets the process's stderr descriptor, which a test
/// cannot observe without permanently redirecting it for every thread in the
/// binary. Splitting the wording out lets a test pin the exact text the drop
/// path emits, as the refused-target diagnostics do, while the counter
/// assertion proves the drop path ran.
///
/// # Examples
///
/// ```rust,ignore
/// let err = log_context::merge_context_values(&oversize_map)
///     .expect_err("an oversize value is refused");
/// assert!(context_drop_diagnostic(&err).contains("invalid context payload"));
/// ```
pub(crate) fn context_drop_diagnostic(error: &log_context::LogContextError) -> String {
    format!("FemtoLogger: dropping record due to invalid context payload: {error}")
}

/// Write the drop diagnostic for `error` to `writer`.
///
/// Split from the logging path so a test can hand it a writer and read back
/// exactly what a caller sees. The production call site passes
/// [`std::io::stderr`] and discards the result: a full or closed diagnostic
/// stream must not fail the logging call that triggered it.
///
/// # Examples
///
/// ```rust,ignore
/// let mut written = Vec::new();
/// report_context_drop(&mut written, &err).expect("a Vec accepts the write");
/// assert_eq!(String::from_utf8(written).unwrap().lines().count(), 1);
/// ```
pub(crate) fn report_context_drop(
    mut writer: impl std::io::Write,
    error: &log_context::LogContextError,
) -> std::io::Result<()> {
    writeln!(writer, "{}", context_drop_diagnostic(error))
}
