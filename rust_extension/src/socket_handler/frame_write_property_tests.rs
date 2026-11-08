//! Properties of the frame write loop across generated call scripts.
//!
//! [`super::tests`] pins one rule at a time with a hand-written script: an
//! interruption is retried, a failure is not, a drain clamps, a spent budget
//! stops the loop. What those cannot show is that the rules *compose* over an
//! arbitrary sequence of partial accepts, interruptions and failures, under
//! each way a worker can be told to stop.
//!
//! Every property is cheap and single-threaded: the sink is scripted in
//! advance, so nothing here waits on a socket or races the host's scheduler.
//! The shutdown deadlines are generous apart from the drain case that names a
//! budget, and that case is decided by arithmetic rather than by how quickly
//! the test thread runs.

use std::{
    io,
    time::{Duration, Instant},
};

use proptest::prelude::*;

use super::FrameWriter;
use super::tests::{ScriptedSink, Step, WRITE_TIMEOUT};
use crate::shutdown_signal::ShutdownSignal;

/// How the worker is told to leave, if it is at all.
#[derive(Debug, Clone, Copy)]
enum Departure {
    /// No shutdown at all: the handler is still running.
    KeepRunning,
    /// Drain towards a deadline this many milliseconds in the future.
    DrainInMillis(u64),
    /// Give up on the frame entirely.
    Abort,
}

impl Departure {
    /// Whether the frame write is expected to be under a shutdown budget.
    ///
    /// This is what makes the "nothing starts once the budget is spent" check
    /// conditional: while the handler runs, the frame's own clock is not read
    /// at all, and a frame that began long ago must still get its full
    /// per-call timeout. Gating on this bool is the property's statement of
    /// that asymmetry rather than a carve-out from it.
    fn is_stopping(self) -> bool {
        !matches!(self, Self::KeepRunning)
    }
}

/// A generated call script: accept some bytes, be interrupted, or break.
fn any_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0_usize..=8).prop_map(Step::Accept),
        Just(Step::Interrupted),
        prop_oneof![
            Just(io::ErrorKind::BrokenPipe),
            Just(io::ErrorKind::ConnectionReset),
        ]
        .prop_map(Step::Fail),
    ]
}

/// A signal in the phase `departure` names, and the drain budget it carries.
///
/// The budget is returned only for a real drain, because it is the ceiling the
/// clamp assertions compare against. `Abort` deliberately reports `None`: its
/// budget is never consulted, since an aborted signal answers `None` to every
/// budget query.
fn signal_for(departure: Departure) -> (ShutdownSignal, Option<Duration>) {
    let signal = ShutdownSignal::new();
    let budget = match departure {
        Departure::KeepRunning => None,
        Departure::DrainInMillis(millis) => {
            let budget = Duration::from_millis(millis);
            signal.begin_shutdown(Instant::now() + budget);
            Some(budget)
        }
        Departure::Abort => {
            signal.begin_shutdown(Instant::now() + Duration::from_secs(30));
            signal.abort();
            None
        }
    };
    (signal, budget)
}

/// Assert the loop never rewound, repeated or skipped a byte.
///
/// Each call must have been offered exactly the frame from the offset the
/// accepted bytes have reached, so the acceptances form one contiguous prefix.
/// An interrupted call transfers nothing, which is why the check runs over the
/// *offered* buffers rather than the accepted bytes alone.
fn check_offsets(frame: &[u8], sink: &ScriptedSink) -> Result<(), TestCaseError> {
    let mut offset = 0_usize;
    for (index, (offered, accepted)) in sink.calls.iter().enumerate() {
        let accepted = *accepted;
        prop_assert!(
            frame
                .get(offset..)
                .is_some_and(|rest| rest.starts_with(offered)),
            "call {index} at offset {offset} was offered {:?}, which is not \
             the frame from that offset",
            offered.len()
        );
        prop_assert!(
            accepted <= offered.len(),
            "call {index} accepted {accepted} of the {} bytes offered",
            offered.len()
        );
        offset += accepted;
    }
    prop_assert_eq!(
        offset,
        sink.accepted.len(),
        "the accepted counts and the accepted bytes must agree"
    );
    prop_assert!(
        frame.starts_with(&sink.accepted),
        "the sink accepted {} bytes, which are not a prefix of the {} byte frame",
        sink.accepted.len(),
        frame.len()
    );
    Ok(())
}

/// Assert every timeout handed over was bounded, and by the right ceiling.
///
/// Three rules share one pass over the recorded timeouts because they are
/// statements about the same list: a zero would mean "block forever", a drain
/// may not hand out more than its own deadline leaves, and a running handler
/// may not shrink the configured per-call ceiling at all.
fn check_budgets(
    departure: Departure,
    drain_budget: Option<Duration>,
    sink: &ScriptedSink,
) -> Result<(), TestCaseError> {
    prop_assert!(
        sink.timeouts.iter().all(|timeout| !timeout.is_zero()),
        "every call needs a non-zero budget, got {:?}",
        sink.timeouts
    );
    if let Some(budget) = drain_budget {
        prop_assert!(
            sink.timeouts.iter().all(|timeout| *timeout <= budget),
            "a call may not outlast the {:?} drain budget, got {:?}",
            budget,
            sink.timeouts
        );
    } else if !departure.is_stopping() {
        prop_assert!(
            sink.timeouts
                .iter()
                .all(|timeout| *timeout == WRITE_TIMEOUT),
            "a running handler gives every call the configured timeout, got {:?}",
            sink.timeouts
        );
    }
    Ok(())
}

/// Assert a frame past its budget under a shutdown never reaches the wire.
///
/// The mirror image of the running case in [`check_budgets`]: while the frame
/// clock is consulted, a frame whose whole budget is already spent must make no
/// call and report `TimedOut` rather than write a partial frame.
fn check_spent_budget(
    departure: Departure,
    multiple: u32,
    sink: &ScriptedSink,
    result: &io::Result<()>,
) -> Result<(), TestCaseError> {
    if !departure.is_stopping() || multiple == 0 {
        return Ok(());
    }
    prop_assert!(
        sink.timeouts.is_empty(),
        "a frame that started {:?} ago, past its {:?} budget, must not make a \
         call, but made {}",
        WRITE_TIMEOUT * multiple,
        WRITE_TIMEOUT,
        sink.timeouts.len()
    );
    prop_assert_eq!(
        result.as_ref().err().map(io::Error::kind),
        Some(io::ErrorKind::TimedOut),
        "an exhausted frame budget must report TimedOut, got {:?}",
        result
    );
    Ok(())
}

/// The property that ties the loop's rules together across generated input.
///
/// Four invariants must hold for every generated combination, each checked by
/// one helper above: byte order and count ([`check_offsets`]), no call without
/// a budget and a drain clamp at every call ([`check_budgets`]), and nothing
/// starting once the frame's budget is spent ([`check_spent_budget`]).
///
/// An interrupted call is retried by the loop, so the script can contain more
/// steps than there are bytes; that is deliberate, and is why the byte-order
/// check is expressed as "a prefix" rather than "the whole frame".
#[test]
fn the_frame_loop_preserves_order_and_never_writes_without_a_budget() {
    let any_departure = prop_oneof![
        Just(Departure::KeepRunning),
        (0_u64..=120).prop_map(Departure::DrainInMillis),
        Just(Departure::Abort),
    ];

    proptest!(|(
        script in prop::collection::vec(any_step(), 0..24),
        frame in prop::collection::vec(any::<u8>(), 0..64),
        departure in any_departure,
        // `0..=4` spans both sides of the frame clock: at 0 the frame is
        // fresh, so a drain's clamp is what decides each call's timeout; from
        // 1 up the frame has spent its whole budget already, so the loop must
        // start nothing.
        multiple in 0_u32..=4,
    )| {
        let (signal, drain_budget) = signal_for(departure);
        let mut sink = ScriptedSink::new(script);
        let mut writer = FrameWriter {
            connection: &mut sink,
            write_timeout: WRITE_TIMEOUT,
            signal: &signal,
            started: Instant::now() - WRITE_TIMEOUT * multiple,
        };

        let result = writer.write_frame(&frame);

        check_offsets(&frame, &sink)?;
        check_budgets(departure, drain_budget, &sink)?;
        check_spent_budget(departure, multiple, &sink, &result)?;
    });
}
