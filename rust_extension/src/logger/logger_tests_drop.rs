//! Tests for what `FemtoLogger::drop` waits for and what it must not wait on.
//!
//! A logger's drop drains its shard and joins nothing, so the two things worth
//! asserting are that a record logged immediately before the drop is still
//! delivered, and that the drop does not hold a lock the record's own path
//! needs.

use super::logger_tests_helpers::{CollectingHandler, collected_messages};
use super::*;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a drop that wrongly returned early is given to be observed.
const OBSERVATION_WINDOW: Duration = Duration::from_millis(250);

/// How long the test waits for a handshake the shard must perform.
const HANDOFF_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn drop_delivers_records_queued_before_it() {
    let handler = Arc::new(CollectingHandler::new());
    let logger =
        FemtoLogger::new("drop-drain".to_owned()).expect("test logger should be constructible");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    logger.log(FemtoLevel::Info, "before drop");
    drop(logger);

    assert_eq!(
        collected_messages(&handler),
        vec!["before drop"],
        "a record logged before the drop must still be dispatched"
    );
}

#[test]
fn drop_delivers_every_record_queued_before_it_in_order() {
    let handler = Arc::new(CollectingHandler::new());
    let logger =
        FemtoLogger::new("drop-order".to_owned()).expect("test logger should be constructible");
    logger
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)
        .expect("native collecting handler should register");

    for index in 0..8 {
        logger.log(FemtoLevel::Info, &format!("msg-{index}"));
    }
    drop(logger);

    let expected: Vec<String> = (0..8).map(|index| format!("msg-{index}")).collect();
    assert_eq!(
        collected_messages(&handler),
        expected,
        "the shard must drain a logger's records in the order it queued them"
    );
}

/// The drop must not hold the handler list while it drains.
///
/// `Drop` waits for the shard to finish the records this logger queued, and a
/// queued handler may want the logger's handler list — a handler that inspects
/// its own configuration, or one that re-reads the list, takes that lock. If
/// the drop held it while waiting, the handler could never finish, the barrier
/// would time out, and the record would be lost.
///
/// The probe reads the logger's *own* `handlers` lock, not an unrelated one.
/// That distinction is the whole test: taking some other lock would pass no
/// matter what the drop held, which is what an earlier version of this test
/// did.
///
/// The probe samples the *whole* window rather than stopping at the first
/// success, and that is what makes the result meaningful. The window opens just
/// before the drop is spawned, so its early probes may run before the dropping
/// thread is scheduled at all; a test that stopped at the first success would
/// pass against a drop that holds the list for the rest of the window. A drop
/// that took the list holds it until its barrier resolves, and the handler is
/// blocked for the whole window, so the barrier cannot resolve inside it: the
/// lock is therefore observed held by at least the later probes, and the
/// assertion fails.
///
/// The handler is blocked inside `handle` before the drop starts, so the record
/// is in flight and the shard's worker cannot advance past it. The window is
/// well inside the shard's two-second barrier timeout, so a drop that took the
/// list cannot time out and release it part-way through.
#[test]
fn drop_does_not_hold_the_handler_list_while_draining() {
    /// How long the main thread samples the handler list while the drop drains.
    const PROBE_WINDOW: Duration = Duration::from_millis(250);

    /// Handler that blocks inside `handle` until the test releases it.
    struct BlockingHandler {
        /// Signals that dispatch has started and the record is in flight.
        entered: Sender<()>,
        /// Released by the test once the probe window has elapsed.
        release: Receiver<()>,
    }

    impl FemtoHandlerTrait for BlockingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            // A refused signal means the test stopped waiting before the
            // handler was entered; there is nobody left to tell, so it is
            // reported and the handler still waits for its release below.
            if self.entered.send(()).is_err() {
                log::trace!("BlockingHandler: test left before the handler was entered");
            }
            // A disconnected release sender means the test dropped it without
            // releasing the handler; the wait returns at once, which is the
            // outcome dropping the sender asks for.
            if self.release.recv().is_err() {
                log::trace!("BlockingHandler: release sender dropped before the handler finished");
            }
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let (entered_tx, entered_rx) = bounded::<()>(1);
    let (release_tx, release_rx) = bounded::<()>(1);

    let logger =
        FemtoLogger::new("drop-lock".to_owned()).expect("test logger should be constructible");
    let handlers = Arc::clone(&logger.handlers);
    logger
        .add_handler(Arc::new(BlockingHandler {
            entered: entered_tx,
            release: release_rx,
        }) as Arc<dyn FemtoHandlerTrait>)
        .expect("native blocking test handler should register");

    logger.log(FemtoLevel::Info, "guarded");
    entered_rx.recv_timeout(Duration::from_secs(5)).expect(
        "the record must reach the handler before the drop starts, so the drop \
         has something in flight to wait for",
    );

    let dropper = std::thread::spawn(move || drop(logger));
    let deadline = Instant::now() + PROBE_WINDOW;
    let mut observed_held = false;
    while Instant::now() < deadline {
        // Drop each guard immediately: the probe is about observation, not
        // contention. Holding it would make the next probe fail on our own
        // account.
        observed_held |= handlers.try_write().is_none();
        std::thread::sleep(Duration::from_millis(1));
    }
    release_tx
        .send(())
        .expect("the handler must still be waiting to be released");
    dropper.join().expect("the dropping thread must not panic");

    assert!(
        !observed_held,
        "the drop must not hold the handler list while it drains: the list was \
         observed locked during a {PROBE_WINDOW:?} window in which the drop was \
         waiting for a blocked handler, which only a drop that took the list \
         can cause"
    );
}

/// Handler that blocks in `handle`, then again in its destructor.
struct BlockingDropHandler {
    /// Signals that the record reached the handler.
    entered: Sender<()>,
    /// Keeps `handle` blocked until the test releases the record.
    release_handle: Receiver<()>,
    /// Signals that the record snapshot released the handler.
    drop_started: Sender<()>,
    /// Keeps the destructor blocked until the test releases it.
    release_drop: Receiver<()>,
}

impl FemtoHandlerTrait for BlockingDropHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        let _ = self.entered.send(());
        let _ = self.release_handle.recv();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for BlockingDropHandler {
    fn drop(&mut self) {
        let _ = self.drop_started.send(());
        let _ = self.release_drop.recv();
    }
}

/// Releases the blocked record and destructor even while unwinding.
struct ReleaseGuard {
    /// Releases `handle`.
    handle: Sender<()>,
    /// Releases the destructor.
    destructor: Sender<()>,
}

impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        let _ = self.handle.try_send(());
        let _ = self.destructor.try_send(());
    }
}

/// A logger dropped while a record job still owns its handler snapshot.
struct BlockedSnapshot {
    /// Releases the record, then the destructor.
    release: ReleaseGuard,
    /// Signals that the record snapshot released the handler.
    drop_started: Receiver<()>,
    /// Set once the dropping thread returns.
    drop_finished: Arc<AtomicBool>,
    /// The logger's own in-flight counter, kept alive past the logger's drop.
    ///
    /// The logger owns the budget the counter lives in, so a clone is what
    /// lets the test read the count while the dropping thread holds the logger.
    in_flight: Arc<LoggerBudget>,
    /// The thread that drops the logger.
    dropper: JoinHandle<()>,
}

/// Arrange a logger whose record job is left owning the handler snapshot.
///
/// The logger's attachment is removed and the test's own handle dropped, so the
/// record's snapshot is the handler's last owner and releasing it is what runs
/// the destructor. The dropping thread is started while the record is still
/// blocked in `handle`, so the drop has something in flight to wait for.
fn blocked_record_snapshot() -> BlockedSnapshot {
    let (entered_tx, entered_rx) = bounded::<()>(1);
    let (release_handle_tx, release_handle_rx) = bounded::<()>(1);
    let (drop_started_tx, drop_started_rx) = bounded::<()>(1);
    let (release_drop_tx, release_drop_rx) = bounded::<()>(1);
    // The shard outlives the logger, as a pooled shard does, so the logger
    // borrows it rather than owning it.
    let shard = Box::leak(Box::new(
        Shard::start(0, 2).expect("the drop-order test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("drop-snapshot".to_string(), shard);
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(BlockingDropHandler {
        entered: entered_tx,
        release_handle: release_handle_rx,
        drop_started: drop_started_tx,
        release_drop: release_drop_rx,
    });
    logger
        .add_handler(Arc::clone(&handler))
        .expect("native blocking test handler should register");

    logger.log(FemtoLevel::Info, "guarded");
    entered_rx.recv_timeout(HANDOFF_TIMEOUT).expect(
        "the record must reach the handler before the drop starts, so the job \
         still owns its handler snapshot",
    );
    assert!(
        logger.remove_handler(&handler),
        "the attached handler must be removable by identity"
    );
    drop(handler);

    let release = ReleaseGuard {
        handle: release_handle_tx,
        destructor: release_drop_tx,
    };
    let drop_finished = Arc::new(AtomicBool::new(false));
    let finished = Arc::clone(&drop_finished);
    // Read the counter through a clone taken while the logger is still here:
    // the dropping thread owns the logger from now on, and the budget the
    // counter lives in would otherwise go with it.
    let (_, budget) = logger
        .dispatch_state()
        .expect("the drop-order test logger should own a dispatch state");
    let in_flight = Arc::clone(budget);
    let dropper = std::thread::spawn(move || {
        drop(logger);
        finished.store(true, Ordering::SeqCst);
    });
    BlockedSnapshot {
        release,
        drop_started: drop_started_rx,
        drop_finished,
        in_flight,
        dropper,
    }
}

/// What the blocked drop was observed doing.
struct ObservedDrop {
    /// Records still in flight when the handler's destructor started.
    in_flight_at_destructor: usize,
    /// Whether the drop finished inside [`OBSERVATION_WINDOW`] while the
    /// destructor was blocked.
    finished_early: bool,
    /// Whether the drop finished once the destructor was released.
    finished: bool,
}

/// Release the record, observe the drop, then release the destructor.
fn release_and_observe(snapshot: BlockedSnapshot) -> ObservedDrop {
    let BlockedSnapshot {
        release,
        drop_started,
        drop_finished,
        in_flight,
        dropper,
    } = snapshot;
    release
        .handle
        .send(())
        .expect("the blocked record must still be waiting for its release");
    drop_started.recv_timeout(HANDOFF_TIMEOUT).expect(
        "releasing the record must drop its handler snapshot, which runs the \
         handler's destructor",
    );
    // The destructor blocks until this test releases it, so the count cannot
    // change between this reading and the drop's completion.
    let in_flight_at_destructor = in_flight.in_flight();
    let finished_early = finished_within(&drop_finished, OBSERVATION_WINDOW);
    release
        .destructor
        .send(())
        .expect("the blocked destructor must still be waiting for its release");
    dropper.join().expect("the dropping thread must not panic");
    ObservedDrop {
        in_flight_at_destructor,
        finished_early,
        finished: drop_finished.load(Ordering::SeqCst),
    }
}

/// Whether `flag` is set before `window` elapses, sampling as it waits.
fn finished_within(flag: &AtomicBool, window: Duration) -> bool {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline && !flag.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1));
    }
    flag.load(Ordering::SeqCst)
}

/// The drop must wait for a record job to release its handler snapshot.
///
/// The shard counts a record as in flight until the job has released
/// everything the record owns. Publishing that completion before the handler
/// snapshot is released leaves a window in which a logger's drop returns while
/// the job still holds the last reference to a handler. A caller that drops the
/// logger and then reads a handler's output finds the handler's destructor has
/// not run, so a stream handler's close-and-flush has not happened either. That
/// window is what the hosted `logger_routes_to_multiple_handlers` failure
/// looked like: one handler's output was already written and the other's was
/// still pending when the test asserted.
///
/// The handler blocks inside its destructor, and the record is held in flight
/// by a blocked `handle`, so the test can tell the two orderings apart:
///
/// - releasing the snapshot before publishing completion keeps the record in
///   flight while the destructor runs, so the drop is still waiting when the
///   destructor starts and the logger's counter is still non-zero;
/// - publishing completion first lets the drop return before the destructor
///   starts at all, and leaves the counter at zero when it does.
///
/// The counter is the scheduling-independent half of that comparison. The
/// completion window alone can pass against the wrong ordering whenever the
/// dropping thread is never scheduled inside it, so the count is sampled while
/// the destructor is blocked: it cannot change between that reading and the
/// drop's completion.
#[test]
fn drop_waits_for_the_record_snapshot_release() {
    let observed = release_and_observe(blocked_record_snapshot());

    assert!(
        observed.in_flight_at_destructor > 0,
        "the job published completion before it released its snapshot: the \
         destructor had started while the record was no longer counted in \
         flight, so a logger drop had nothing left to wait for"
    );
    assert!(
        !observed.finished_early,
        "the drop returned while a record job still owned the handler: the \
         destructor had started and the record was still in flight, so only a \
         job that published completion before releasing its snapshot can \
         explain the drop finishing inside {OBSERVATION_WINDOW:?}"
    );
    assert!(
        observed.finished,
        "the drop must finish once the record job has released its snapshot"
    );
}
