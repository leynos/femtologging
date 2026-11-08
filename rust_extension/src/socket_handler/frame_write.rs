//! Writing one frame under the handler's write timeout and shutdown deadline.
//!
//! A socket write timeout bounds one call, not one frame. A frame larger than
//! the send buffer is therefore several calls, and the previous `write_all`
//! spent another full timeout on each without ever re-reading the shutdown
//! state. This module owns that loop so it can re-clamp between calls.
//!
//! Split from `transport.rs` along the seam between opening a connection and
//! using one, which also keeps both files inside the repository's line limit.

use std::{
    io,
    time::{Duration, Instant},
};

use crate::shutdown_signal::ShutdownSignal;

use super::transport::ActiveConnection;

/// The connection operations one frame write needs.
///
/// A trait rather than direct [`ActiveConnection`] calls so the write loop can
/// be driven by a scripted peer. An interrupted system call cannot be provoked
/// on a real socket, and the retry that handles one is a contract that needs a
/// test of its own. [`ActiveConnection`] is the only production implementor.
pub(super) trait FrameSink {
    /// Bound the next write or flush call by `timeout`.
    fn set_write_timeout(&mut self, timeout: Duration) -> io::Result<()>;

    /// Write as much of `buf` as one call accepts, reporting the count.
    ///
    /// A non-empty buffer must never report `Ok(0)`: the write loop reads that
    /// as a peer that has stopped accepting, and would otherwise spin.
    fn write_some(&mut self, buf: &[u8]) -> io::Result<usize>;

    /// Flush what the calls have accepted.
    fn flush(&mut self) -> io::Result<()>;
}

impl FrameSink for ActiveConnection {
    fn set_write_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        ActiveConnection::set_write_timeout(self, timeout)
    }

    fn write_some(&mut self, buf: &[u8]) -> io::Result<usize> {
        ActiveConnection::write_some(self, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        ActiveConnection::flush(self)
    }
}

/// Sending one complete frame on an [`ActiveConnection`] under a deadline.
///
/// Deliberately not a method on the connection: what a frame write has to
/// decide between calls is not a property of the socket. It needs the
/// handler's configured budget, the shutdown signal, and the time the write
/// began, and all three belong to the worker rather than to the transport.
pub(super) struct FrameWriter<'a, S: FrameSink = ActiveConnection> {
    /// Connection the frame is written to.
    pub connection: &'a mut S,
    /// Configured upper bound on one write call; re-clamped per call.
    pub write_timeout: Duration,
    /// Stop and budget source consulted before every call.
    pub signal: &'a ShutdownSignal,
    /// When the first byte of this frame was attempted.
    pub started: Instant,
}

impl<S: FrameSink> FrameWriter<'_, S> {
    /// Write every byte of `frame`, or stop at the first point no budget remains.
    ///
    /// A socket timeout bounds one call, not a frame, and a slow collector
    /// turns one frame into many calls. Owning the loop is what lets shutdown
    /// re-clamp between them: `write_all` would have spent another full timeout
    /// on each partial write without consulting the signal at all.
    ///
    /// While the handler is running, each call simply gets the configured
    /// timeout. Once shutdown begins, the call budget shrinks to whatever is
    /// left of both the frame's clock and the shutdown deadline, so the frame
    /// cannot outlast the deadline.
    ///
    /// A spent budget returns `TimedOut` with the progress made so far in the
    /// message, which is the difference between a collector that is slow and
    /// one that has stopped reading. That error is not a failure of the
    /// connection: the bytes already written are exactly what the collector
    /// saw, and the worker treats the record as a shutdown drop.
    ///
    /// An interrupted call is retried rather than propagated. `write_all`
    /// retried it, and treating it as a failed frame would discard a record
    /// and drop the connection for a signal that transferred nothing and
    /// changed no state.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut writer = FrameWriter { connection: &mut conn, write_timeout, signal, started };
    /// writer.write_frame(b"framed")?;
    /// ```
    pub(super) fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        let mut written = 0_usize;
        while written < frame.len() {
            let Some(budget) = self.remaining_budget() else {
                return Err(frame_budget_spent(written, frame.len()));
            };
            self.connection.set_write_timeout(budget)?;
            match self.connection.write_some(&frame[written..]) {
                Ok(count) => written += count,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err),
            }
        }
        // `flush` can block on the same send queue, so it is the last step and
        // asks for its own budget: during shutdown the loop above may already
        // have spent what the frame began with, and `flush` must not inherit
        // the last call's tiny remainder or start with none left at all.
        let Some(budget) = self.remaining_budget() else {
            return Err(frame_budget_spent(written, frame.len()));
        };
        self.connection.set_write_timeout(budget)?;
        self.connection.flush()
    }

    /// What this call may spend, or `None` when nothing may start.
    ///
    /// While the handler is running there is no shutdown deadline to respect,
    /// so the configured timeout is handed over unchanged and each call gets
    /// the full per-call ceiling. That is the documented contract, and it is
    /// what lets a slow but progressing collector finish a large frame.
    ///
    /// Only once shutdown has begun does the frame's own clock matter. Then
    /// two ceilings apply and neither can outlast the other: the signal's
    /// clamp says what the handler's deadline leaves, and the elapsed time
    /// says what the frame has already spent of its budget. The frame clock
    /// exists to stop one frame outlasting the shutdown deadline; charging it
    /// against ordinary operation instead would turn `write_timeout` into a
    /// whole-frame budget and drop frames that were still making progress.
    ///
    /// "Now" is sampled here and handed to [`spent_within_frame`], which is a
    /// pure function of the frame's start, its budget and that instant. The
    /// clock read stays at the edge of the decision rather than inside it.
    fn remaining_budget(&self) -> Option<Duration> {
        if !self.signal.is_shutting_down() {
            return Some(self.write_timeout);
        }
        let within_frame = spent_within_frame(self.write_timeout, self.started, Instant::now())?;
        self.signal.clamp(within_frame)
    }
}

/// What is left of a frame's own budget as of `now`, or `None` once spent.
///
/// The frame's clock is the shutdown clamp only: it stops one frame outlasting
/// the handler's deadline. Kept as a function of its arguments so the boundary
/// cases - exactly at the deadline, one nanosecond before it - can be tested
/// without racing a real clock.
///
/// # Examples
///
/// ```ignore
/// let budget = Duration::from_millis(50);
/// let started = Instant::now();
/// assert_eq!(spent_within_frame(budget, started, started), Some(budget));
/// assert_eq!(spent_within_frame(budget, started, started + budget), None);
/// ```
pub(super) fn spent_within_frame(
    budget: Duration,
    started: Instant,
    now: Instant,
) -> Option<Duration> {
    budget
        .checked_sub(now.saturating_duration_since(started))
        .filter(|left| !left.is_zero())
}

/// The error produced when a frame write runs out of budget part-way through.
///
/// `TimedOut`, the same kind `budget_spent` uses, so the worker's existing
/// handling applies; the message carries the progress because "wrote 1 MiB of
/// 4 MiB" and "wrote nothing" are different collectors to go and look at.
fn frame_budget_spent(written: usize, total: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "shutdown budget spent after writing {written} of {total} frame bytes; \
             the collector is not keeping up"
        ),
    )
}

#[cfg(test)]
#[path = "frame_write_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "frame_write_property_tests.rs"]
mod property_tests;
