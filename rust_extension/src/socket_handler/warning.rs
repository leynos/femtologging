//! The socket handler's dropped-record warnings.
//!
//! The handler refuses records at its queue and its worker discovers drops
//! mid-delivery, each counting into its own [`RateLimitedWarner`]. These
//! helpers are therefore shared by both halves rather than owned by either: one
//! pair for a drop that may be suppressed until the interval elapses, and one
//! for a report that must be made now because there is no later chance.
//!
//! They live beside the handler rather than inside it because the handler
//! module is at its size limit, and the wording is one cohesive unit.

use crate::rate_limited_warner::RateLimitedWarner;

/// Record a drop and emit the callback only when the warning interval permits.
pub(super) fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
}

/// Report pending drops without waiting for the warning interval.
///
/// Both the handler (a refused queue) and its worker (serialization,
/// connection or write failures) count drops, so both use this one report.
/// A suppression window must not carry a count past the last chance to say it:
/// on close, flush, or worker exit there is no later warning.
pub(super) fn flush_drops(warner: &RateLimitedWarner) {
    warner.flush(|count| {
        log::warn!("FemtoSocketHandler dropped {count} records since the last report");
    });
}

/// Record a drop and report the cumulative count, naming this drop's cause as
/// the one that triggered the report.
///
/// One warner counts drops from several causes — the handler's sees queue-full,
/// shutdown and closed refusals, the worker's sees serialization, connection
/// and write failures — so a due warning can carry drops from several causes.
/// Saying the count happened for this call's reason would misattribute the
/// earlier ones, and nothing here can know which of them was most recent:
/// another thread may record a drop between this one and the report. The
/// wording therefore names only the cause whose refusal asked for the warning.
pub(super) fn warn_drops_naming_cause(warner: &RateLimitedWarner, cause: &str) {
    warn_drops(warner, |count| warn_cumulative_drops(count, cause));
}

/// Report the pending cumulative count now, naming the triggering cause.
///
/// The flush equivalent of [`warn_drops_naming_cause`], for paths that cannot
/// wait for the interval and whose count may still hold drops from earlier
/// causes.
pub(super) fn flush_drops_naming_cause(warner: &RateLimitedWarner, cause: &str) {
    warner.flush(|count| warn_cumulative_drops(count, cause));
}

/// Render the one cumulative drop warning the naming helpers emit.
///
/// Visible to the handler's tests so a controlled race between two causes can
/// assert on the message the production helper would produce for its winner.
pub(super) fn warn_cumulative_drops(count: u64, cause: &str) {
    log::warn!(
        concat!(
            "FemtoSocketHandler dropped {} records since the last report; ",
            "triggering cause: {}"
        ),
        count,
        cause
    );
}
