//! The stream handler's one dropped-record warning.
//!
//! Every refusal path and every explicit report speaks through
//! [`warn_dropped`], so the wording lives in one place. It sits beside the
//! handler rather than inside it because the handler module is at its size
//! limit, and this is the cohesive unit that can leave without splitting the
//! shutdown logic from the state it closes.

use log::warn;

/// Report that `count` records were dropped by this handler.
///
/// Shaped to be passed directly as the reporting closure of
/// [`RateLimitedWarner::warn_if_due`] and [`RateLimitedWarner::flush`], so all
/// five call sites report through this one message: the three refusal paths in
/// [`FemtoHandlerTrait::handle`], [`FemtoHandlerTrait::flush`], and
/// [`super::FemtoStreamHandler::take_shutdown`].
///
/// The rendered text ends with `log records dropped since the last report`,
/// which `tests/stream_handler_logging_tests.rs` strips to read the count back.
/// That test cannot import this function, so it holds its own copy of the
/// suffix: changing the wording here means changing it there too, or the count
/// the test asserts will silently stop being parsed. "Since the last report"
/// rather than "in the last interval" because a suppressed interval carries
/// its drops into the next report, which can cover more than one interval.
///
/// # Examples
///
/// ```ignore
/// warner.warn_if_due(warn_dropped);
/// ```
///
/// [`FemtoHandlerTrait::handle`]: crate::handler::FemtoHandlerTrait::handle
/// [`FemtoHandlerTrait::flush`]: crate::handler::FemtoHandlerTrait::flush
/// [`RateLimitedWarner::warn_if_due`]: crate::rate_limited_warner::RateLimitedWarner::warn_if_due
/// [`RateLimitedWarner::flush`]: crate::rate_limited_warner::RateLimitedWarner::flush
pub(super) fn warn_dropped(count: u64) {
    warn!("FemtoStreamHandler: {count} log records dropped since the last report");
}

#[cfg(test)]
mod tests {
    //! Tests for the stream handler's shutdown reporting.

    use crate::formatter::DefaultFormatter;
    use crate::stream_handler::FemtoStreamHandler;

    /// The Python `close()` path calls `take_shutdown` directly, so the flush
    /// must live there: the Python object stays alive after `handler.close()`,
    /// and nothing else reports what was still pending.
    #[test]
    fn take_shutdown_reports_pending_drops() {
        let handler = FemtoStreamHandler::new(std::io::sink(), DefaultFormatter);
        handler.warner.record_drop();
        assert_eq!(handler.warner.pending_drops(), 1);

        let shutdown = handler.take_shutdown();
        assert_eq!(
            handler.warner.pending_drops(),
            0,
            "shutdown must report the drops still pending",
        );
        shutdown.wait();
    }
}
