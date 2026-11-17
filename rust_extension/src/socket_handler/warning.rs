//! The socket handler's dropped-record warnings.
//!
//! The handler refuses records at its queue and its worker discovers drops
//! mid-delivery, but both count into one [`RateLimitedWarner`]. These helpers
//! are therefore shared by both halves rather than owned by either: one pair
//! for a drop that may be suppressed until the interval elapses, and one for a
//! report that must be made now because there is no later chance.
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
/// the latest rather than as the reason for the whole count.
///
/// The handler and the worker share one counter for every drop they see, so a
/// due warning can carry drops from several causes. Saying the count happened
/// for this call's reason would misattribute the earlier ones; the wording
/// therefore names only the most recent cause.
pub(super) fn warn_drops_naming_latest(warner: &RateLimitedWarner, cause: &str) {
    warn_drops(warner, |count| warn_cumulative_drops(count, cause));
}

/// Report the pending cumulative count now, naming the most recent cause.
///
/// The flush equivalent of [`warn_drops_naming_latest`], for paths that cannot
/// wait for the interval and whose count may still hold drops from earlier
/// causes.
pub(super) fn flush_drops_naming_latest(warner: &RateLimitedWarner, cause: &str) {
    warner.flush(|count| warn_cumulative_drops(count, cause));
}

/// Render the one cumulative drop warning the naming helpers emit.
fn warn_cumulative_drops(count: u64, cause: &str) {
    log::warn!(
        concat!(
            "FemtoSocketHandler dropped {} records since the last report; ",
            "latest cause: {}"
        ),
        count,
        cause
    );
}
