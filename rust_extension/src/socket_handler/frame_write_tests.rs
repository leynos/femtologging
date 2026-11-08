//! Contracts for the frame write loop that a real socket cannot exercise.
//!
//! Two of the loop's rules are invisible from a socket: an interrupted system
//! call has to be retried rather than reported as a failed frame, and the
//! configured timeout is a per-call ceiling during ordinary operation rather
//! than a budget for the whole frame. Neither can be provoked on loopback - an
//! `EINTR` needs a signal to land inside one specific `write` call - so the
//! loop is driven by a scripted [`FrameSink`] instead.

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use rstest::rstest;

use crate::shutdown_signal::ShutdownSignal;
use crate::socket_handler::frame_write::{FrameSink, FrameWriter, spent_within_frame};

/// The write timeout the scripted cases configure.
pub(super) const WRITE_TIMEOUT: Duration = Duration::from_millis(50);

/// What one scripted `write_some` call does.
#[derive(Debug, Clone, Copy)]
pub(super) enum Step {
    /// Accept at most this many bytes, as a partial write does.
    Accept(usize),
    /// Report an interrupted call, transferring nothing.
    Interrupted,
    /// Fail with this kind, as a broken connection does.
    Fail(io::ErrorKind),
}

/// A connection whose every call is decided in advance.
///
/// Each scripted step is spent once; a call with an empty script accepts
/// everything still offered, which is how a test expresses "and then the rest
/// goes through".
pub(super) struct ScriptedSink {
    /// Steps still to be performed, in call order.
    steps: VecDeque<Step>,
    /// Write timeouts the loop set, in call order.
    pub(super) timeouts: Vec<Duration>,
    /// The timeout in force when `flush` was reached, if it was.
    flush_timeout: Option<Duration>,
    /// Every byte the sink accepted, in order.
    pub(super) accepted: Vec<u8>,
    /// What each call was offered and how much of it was accepted.
    ///
    /// Recorded so a property test can check the loop's offsets, not only the
    /// bytes that survived: an interrupted call transfers nothing, so a record
    /// of acceptances alone cannot tell "offered the right bytes at the right
    /// offset" from "offered them again".
    pub(super) calls: Vec<(Vec<u8>, usize)>,
}

impl ScriptedSink {
    /// A sink that performs `steps` and then accepts the remainder.
    pub(super) fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: steps.into(),
            timeouts: Vec::new(),
            flush_timeout: None,
            accepted: Vec::new(),
            calls: Vec::new(),
        }
    }
}

impl FrameSink for ScriptedSink {
    fn set_write_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.timeouts.push(timeout);
        Ok(())
    }

    fn write_some(&mut self, buf: &[u8]) -> io::Result<usize> {
        let step = self.steps.pop_front().unwrap_or(Step::Accept(buf.len()));
        match step {
            Step::Accept(most) => {
                let count = most.min(buf.len());
                self.accepted.extend_from_slice(&buf[..count]);
                self.calls.push((buf.to_vec(), count));
                Ok(count)
            }
            Step::Interrupted => {
                self.calls.push((buf.to_vec(), 0));
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "a signal arrived during the write",
                ))
            }
            Step::Fail(kind) => {
                self.calls.push((buf.to_vec(), 0));
                Err(io::Error::new(kind, "the scripted connection failed"))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_timeout = self.timeouts.last().copied();
        Ok(())
    }
}

/// An interrupted write is retried, not reported as a failed frame.
///
/// `write_all` retried `ErrorKind::Interrupted` internally, so treating one as
/// a failed frame discards a record and drops the connection for a signal that
/// transferred nothing and changed no state. The loop must also re-check the
/// budget after one, which is why the retry is a new iteration rather than an
/// inner loop of its own.
#[rstest]
fn an_interrupted_write_is_retried_without_losing_bytes() {
    let signal = ShutdownSignal::new();
    let mut sink = ScriptedSink::new(vec![Step::Interrupted]);
    let frame = b"framed";
    let mut writer = FrameWriter {
        connection: &mut sink,
        write_timeout: WRITE_TIMEOUT,
        signal: &signal,
        started: Instant::now(),
    };

    writer
        .write_frame(frame)
        .expect("an interrupted call must not fail the frame");

    assert_eq!(
        sink.accepted, frame,
        "the retried frame must arrive intact after the interruption"
    );
    assert!(
        sink.flush_timeout.is_some(),
        "the frame was complete, so it must still be flushed"
    );
}

/// An ordinary write error still fails the frame at once.
///
/// The retry above must not be generalised into a blind loop: anything other
/// than `Interrupted` is the connection failing, and the worker's existing
/// recovery needs to see it.
#[rstest]
fn a_broken_connection_is_not_retried() {
    let signal = ShutdownSignal::new();
    let mut sink = ScriptedSink::new(vec![Step::Fail(io::ErrorKind::BrokenPipe)]);
    let mut writer = FrameWriter {
        connection: &mut sink,
        write_timeout: WRITE_TIMEOUT,
        signal: &signal,
        started: Instant::now(),
    };

    let err = writer
        .write_frame(b"framed")
        .expect_err("a broken pipe must fail the frame");

    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    assert!(
        sink.flush_timeout.is_none(),
        "a frame that never completed must not be flushed"
    );
}

/// While the handler runs, every call gets the configured timeout in full.
///
/// The frame clock exists to stop one frame outlasting a *shutdown* deadline.
/// Charging it against ordinary operation would turn `write_timeout` into a
/// whole-frame budget and drop frames that were still making progress, which
/// is why the writer is started a full budget in the past here: a running
/// handler must not read that clock at all.
#[rstest]
fn a_running_handler_gives_every_call_the_configured_timeout() {
    let signal = ShutdownSignal::new();
    let mut sink = ScriptedSink::new(vec![Step::Accept(2)]);
    let frame = b"framed";
    let started = Instant::now() - Duration::from_secs(1);
    let mut writer = FrameWriter {
        connection: &mut sink,
        write_timeout: WRITE_TIMEOUT,
        signal: &signal,
        started,
    };

    writer
        .write_frame(frame)
        .expect("a slow frame must still finish while the handler runs");

    assert_eq!(sink.accepted, frame);
    assert!(
        sink.timeouts.len() >= 2,
        "the partial write should have made the loop iterate"
    );
    assert!(
        sink.timeouts
            .iter()
            .all(|timeout| *timeout == WRITE_TIMEOUT),
        "each call must get the configured timeout, got {:?}",
        sink.timeouts
    );
}

/// Once draining, the frame clock stops a frame outlasting the deadline.
///
/// Both the signal's clamp and the frame's own clock now apply, and a frame
/// that has already spent its budget must not start another call - the
/// difference between a collector that is slow and one that has stopped
/// reading.
#[rstest]
fn a_drain_that_has_spent_the_frame_budget_stops_before_writing() {
    let signal = ShutdownSignal::new();
    signal.begin_shutdown(Instant::now() + Duration::from_secs(30));
    let mut sink = ScriptedSink::new(Vec::new());
    let started = Instant::now() - Duration::from_secs(1);
    let mut writer = FrameWriter {
        connection: &mut sink,
        write_timeout: WRITE_TIMEOUT,
        signal: &signal,
        started,
    };

    let err = writer
        .write_frame(b"framed")
        .expect_err("an exhausted frame budget must stop the write");

    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(
        sink.timeouts.is_empty(),
        "no call may start without budget, got {:?}",
        sink.timeouts
    );
    assert!(
        err.to_string().contains("after writing 0 of"),
        "the error must name the progress made, got {err}"
    );
}

/// A drain clamps each call to what the shutdown deadline leaves.
///
/// The configured timeout is only a ceiling once the signal is draining; the
/// signal's remainder is the tighter of the two whenever the deadline is near,
/// and handing over the configured timeout instead would let a frame outlast
/// the handler's budget.
#[rstest]
fn a_drain_clamps_each_call_to_the_shutdown_budget() {
    const DRAIN_BUDGET: Duration = Duration::from_millis(200);
    let signal = ShutdownSignal::new();
    signal.begin_shutdown(Instant::now() + DRAIN_BUDGET);
    let mut sink = ScriptedSink::new(vec![Step::Accept(2)]);
    let mut writer = FrameWriter {
        connection: &mut sink,
        write_timeout: WRITE_TIMEOUT,
        signal: &signal,
        started: Instant::now(),
    };

    writer
        .write_frame(b"framed")
        .expect("a frame well inside the drain budget should still be written");

    assert_eq!(sink.accepted, b"framed");
    assert!(
        !sink.timeouts.is_empty(),
        "the frame should have made calls"
    );
    assert!(
        sink.timeouts.iter().all(|timeout| *timeout <= DRAIN_BUDGET),
        "no call may outlast the drain deadline, got {:?}",
        sink.timeouts
    );
}

/// A frame that has just started still has its whole budget.
///
/// The boundary the frame clock exists to police: the frame's budget is what
/// the shutdown deadline may spend on it, so at the instant the first byte was
/// attempted nothing is spent yet.
#[rstest]
fn a_freshly_started_frame_has_its_whole_budget() {
    let started = Instant::now();

    assert_eq!(
        spent_within_frame(WRITE_TIMEOUT, started, started),
        Some(WRITE_TIMEOUT),
        "no time has passed, so nothing is spent"
    );
}

/// Spending exactly the budget leaves nothing, rather than a zero.
///
/// A zero remainder must not be handed over: as a socket timeout it means "no
/// timeout", which is the unbounded wait the budget exists to forbid.
#[rstest]
fn a_frame_that_has_spent_its_budget_reports_nothing() {
    let started = Instant::now();

    assert_eq!(
        spent_within_frame(WRITE_TIMEOUT, started, started + WRITE_TIMEOUT),
        None,
        "a fully spent budget must not be reported as a zero remainder"
    );
}

/// Time beyond the budget is spent, not a wrapped-around remainder.
///
/// A deadline that passed some time ago must stay spent. An implementation
/// that subtracted in the wrong direction, or that used a wrapping duration,
/// would report a large budget here and let the frame run past the deadline.
#[rstest]
fn a_frame_past_its_budget_stays_spent() {
    let started = Instant::now();

    assert_eq!(
        spent_within_frame(WRITE_TIMEOUT, started, started + WRITE_TIMEOUT * 3),
        None,
        "a frame long past its budget must not regain one"
    );
}

/// A partially spent frame reports exactly what is left.
///
/// The middle of the range is where the write loop actually operates: it
/// re-asks for the budget between calls, and the answer has to shrink by
/// exactly the time already spent or the deadline drifts.
#[rstest]
fn a_partly_spent_frame_reports_the_remainder() {
    let started = Instant::now();
    let spent = WRITE_TIMEOUT / 4;

    assert_eq!(
        spent_within_frame(WRITE_TIMEOUT, started, started + spent),
        Some(WRITE_TIMEOUT - spent),
        "the remainder must be the budget less exactly the time spent"
    );
}
