//! Deterministic deadline tests for the shared handler flush queue wait.
//!
//! Each test stalls one half of a real bounded channel — a full queue for
//! admission, an unanswered marker for acknowledgement — and scripts the clock
//! so the supplied deadline, not the wall clock, decides how long the wait can
//! last.

use std::thread;
use std::time::{Duration, Instant};

use super::{QueueFlushFailure, flush_command_until};
use crate::sync::{Sender, bounded};

/// Command shape used to exercise the helper's two queue waits.
enum TestCommand {
    /// Carries the acknowledgement channel answered by the test worker.
    Flush(Sender<()>),
}

/// Time source that returns one scripted offset per sample.
struct ScriptedClock {
    /// Time every sample is measured from.
    start: Instant,
    /// Offsets returned in order; the final offset repeats.
    offsets: Vec<Duration>,
    /// Number of samples requested so far.
    samples: usize,
}

impl ScriptedClock {
    fn new(offsets_ms: &[u64]) -> Self {
        Self {
            start: Instant::now(),
            offsets: offsets_ms
                .iter()
                .copied()
                .map(Duration::from_millis)
                .collect(),
            samples: 0,
        }
    }

    /// Deadline used by the tests, chosen so only the scripted offsets expire it.
    fn deadline(&self) -> Instant {
        self.start + Duration::from_secs(30)
    }

    fn now(&mut self) -> Instant {
        let index = self.samples.min(self.offsets.len().saturating_sub(1));
        self.samples += 1;
        self.start + self.offsets[index]
    }
}

/// Send one `TestCommand::Flush` through `flush_command_until`.
///
/// The helper owns the acknowledgement channel because each handler's queue
/// type differs; this mirrors how a handler hands the helper its queue.
fn flush_test_command(
    tx: &Sender<TestCommand>,
    deadline: Instant,
    now: &mut dyn FnMut() -> Instant,
) -> Result<(), QueueFlushFailure> {
    flush_command_until(deadline, now, |budget| {
        let (ack_tx, ack_rx) = bounded(1);
        tx.send_timeout(TestCommand::Flush(ack_tx), budget).ok()?;
        Some(move |budget| ack_rx.recv_timeout(budget).ok())
    })
}

/// A full queue must refuse the marker once the deadline has no budget left.
#[test]
fn a_full_queue_is_bounded_by_the_supplied_deadline() {
    let (tx, _rx) = bounded::<TestCommand>(1);
    let (filler, _filler_ack) = bounded(1);
    tx.try_send(TestCommand::Flush(filler))
        .expect("the single queue slot should start empty");

    let mut clock = ScriptedClock::new(&[30_000]);
    let deadline = clock.deadline();
    let started = Instant::now();
    let outcome = {
        let mut now = || clock.now();
        flush_test_command(&tx, deadline, &mut now)
    };

    assert_eq!(outcome, Err(QueueFlushFailure::Enqueue));
    assert_eq!(
        clock.samples, 1,
        "a refused admission must not sample the clock for an acknowledgement it never sent"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "queue admission must stop at the supplied deadline, not wait for room"
    );
}

/// A worker that never answers must not extend the caller past the deadline.
#[test]
fn a_stalled_acknowledgement_is_bounded_by_the_supplied_deadline() {
    // The receiver stays alive but is never read, so the worker is stalled.
    let (tx, _rx) = bounded::<TestCommand>(1);
    let mut clock = ScriptedClock::new(&[0, 30_000]);
    let deadline = clock.deadline();
    let started = Instant::now();
    let outcome = {
        let mut now = || clock.now();
        flush_test_command(&tx, deadline, &mut now)
    };

    assert_eq!(outcome, Err(QueueFlushFailure::Acknowledgement));
    assert_eq!(
        clock.samples, 2,
        "admission and acknowledgement must each take the budget left when they begin"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "acknowledgement must stop at the supplied deadline, not the handler timeout"
    );
}

/// A worker that answers before the deadline confirms the flush.
#[test]
fn a_confirmed_flush_returns_its_acknowledgement() {
    let (tx, rx) = bounded::<TestCommand>(1);
    let worker = thread::spawn(move || {
        if let Ok(TestCommand::Flush(ack)) = rx.recv() {
            let _ = ack.send(());
        }
    });

    let mut clock = ScriptedClock::new(&[0, 0]);
    let deadline = clock.deadline();
    let outcome = {
        let mut now = || clock.now();
        flush_test_command(&tx, deadline, &mut now)
    };

    assert_eq!(outcome, Ok(()));
    worker.join().expect("the test worker should exit cleanly");
}
