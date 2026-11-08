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

/// The property that ties the loop's rules together across generated input.
///
/// Four invariants must hold for every generated combination:
///
/// 1. **Byte order and count.** Every buffer handed to the sink continues
///    exactly where the previous one ended, and all of them together are an
///    in-order prefix of the frame. A loop that rewound, repeated or skipped an
///    offset would corrupt the stream, which is invisible in the scripted cases
///    because the sink there only ever checks what it was given.
/// 2. **No call without budget.** Every timeout handed to the sink is non-zero.
///    A zero would mean "block forever", the unbounded wait the budget exists
///    to forbid.
/// 3. **A drain clamp holds at every call.** Once draining, no call may outlast
///    what the deadline leaves, whatever the configured timeout is.
/// 4. **Nothing starts after the budget is spent.** Under a shutdown, a frame
///    that began at least one whole write timeout ago gets no further call and
///    reports `TimedOut` without touching the sink at all.
///
/// An interrupted call is retried by the loop, so the script can contain more
/// steps than there are bytes; that is deliberate, and is why the byte-order
/// check is expressed as "a prefix" rather than "the whole frame".
#[test]
fn the_frame_loop_preserves_order_and_never_writes_without_a_budget() {
    // A drain budget that spans both sides of the configured timeout: below it,
    // the signal's remainder is the tighter ceiling and the clamp is visible;
    // above it, the frame's own clock is. A budget of zero exercises the
    // already-expired case, where nothing may start at all.
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
        // start nothing. The multiplier is deliberately not `..0`-based: a
        // frame that began even one whole write timeout ago is spent
        // arithmetically, not by however fast the host happens to run.
        multiple in 0_u32..=4,
    )| {
        let signal = ShutdownSignal::new();
        let drain_budget = match departure {
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
        let mut sink = ScriptedSink::new(script);
        let mut writer = FrameWriter {
            connection: &mut sink,
            write_timeout: WRITE_TIMEOUT,
            signal: &signal,
            started: Instant::now() - WRITE_TIMEOUT * multiple,
        };

        let result = writer.write_frame(&frame);

        // 1. The loop never rewound, repeated or skipped a byte. Each call must
        //    have been offered exactly the frame from the offset the accepted
        //    bytes have reached, so the offered buffers are contiguous and the
        //    acceptances are an in-order prefix.
        let mut offset = 0_usize;
        for (index, (offered, accepted)) in sink.calls.iter().enumerate() {
            let accepted = *accepted;
            prop_assert!(
                frame.get(offset..).is_some_and(|rest| rest.starts_with(offered)),
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
            "the sink accepted {} bytes, which are not a prefix of the {} byte \
             frame",
            sink.accepted.len(),
            frame.len()
        );

        // 2. No call was ever handed an unbounded (zero) timeout.
        prop_assert!(
            sink.timeouts.iter().all(|timeout| !timeout.is_zero()),
            "every call needs a non-zero budget, got {:?}",
            sink.timeouts
        );

        // 3. While draining, no call outlasts what the deadline leaves.
        if let Some(budget) = drain_budget {
            prop_assert!(
                sink.timeouts.iter().all(|timeout| *timeout <= budget),
                "a call may not outlast the {:?} drain budget, got {:?}",
                budget,
                sink.timeouts
            );
        }

        // 4. A frame already past its budget never reaches the connection,
        //    *when a budget applies at all*. While the handler runs the frame
        //    clock is not consulted, so the same frame must still be written.
        if departure.is_stopping() && multiple >= 1 {
            prop_assert!(
                sink.timeouts.is_empty(),
                "a frame that started {:?} ago, past its {:?} budget, must not \
                 make a call, but made {}",
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
        }
        if !departure.is_stopping() {
            // The mirror image: a running handler must ignore the frame clock,
            // which is what lets a slow but progressing collector finish.
            prop_assert!(
                sink.timeouts.iter().all(|timeout| *timeout == WRITE_TIMEOUT),
                "a running handler gives every call the configured timeout, \
                 got {:?}",
                sink.timeouts
            );
        }
    });
}
