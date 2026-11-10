//! Flush-deadline tests for the stream handler.
//!
//! Both tests stall the worker inside the writer, so nothing can answer the
//! handler's flush marker until the test releases the writer. That makes two
//! budget properties deterministic: an acknowledgement whose budget was
//! already spent by admission returns without waiting, and a marker that never
//! enters a full queue hands its deferred drop count back to the warner.

use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use crate::formatter::DefaultFormatter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::rate_limited_warner::RateLimitedWarner;
use crate::stream_handler::{FemtoStreamHandler, HandlerConfig};
use crate::test_utils::flush_budget::{ScriptedClock, WarningClock};

use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};

/// Writer whose `flush` blocks until the test releases it.
///
/// `write` succeeds immediately so the record that occupies the worker is
/// consumed from the queue; only the flush keeps the worker stalled.
#[derive(Clone)]
struct BlockingFlushWriter {
    /// State shared with every clone of the writer.
    state: Arc<BlockingFlushState>,
}

/// Coordination shared between the writer and the controlling test.
struct BlockingFlushState {
    /// Number assigned to each flush invocation.
    flushes: Mutex<usize>,
    /// Highest flush number the test has released.
    released: Mutex<usize>,
    /// Wakes blocked flush calls when the test advances `released`.
    release: Condvar,
    /// Reports each started flush to the controlling test.
    started: mpsc::Sender<usize>,
}

impl BlockingFlushWriter {
    /// Build a writer that reports each flush entry to `started`.
    fn new(started: mpsc::Sender<usize>) -> Self {
        Self {
            state: Arc::new(BlockingFlushState {
                flushes: Mutex::new(0),
                released: Mutex::new(0),
                release: Condvar::new(),
                started,
            }),
        }
    }

    /// Release every blocked flush call.
    fn release_all(&self) {
        *self
            .state
            .released
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = usize::MAX;
        self.state.release.notify_all();
    }
}

impl Write for BlockingFlushWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let flush_number = {
            let mut flushes = self
                .state
                .flushes
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *flushes += 1;
            *flushes
        };
        self.state.started.send(flush_number).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "flush start receiver disconnected",
            )
        })?;

        let mut released = self
            .state
            .released
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while *released < flush_number {
            released = self
                .state
                .release
                .wait(released)
                .unwrap_or_else(PoisonError::into_inner);
        }
        Ok(())
    }
}

/// Build a record for the tests.
fn record(message: &str) -> FemtoLogRecord {
    FemtoLogRecord::new("flush-budget", FemtoLevel::Info, message)
}

/// A budget already spent on admission leaves the acknowledgement nothing to
/// wait with, even though the handler's own timeout would still allow a wait.
#[test]
fn acknowledgement_stops_at_the_shared_deadline() {
    let (started_tx, started_rx) = mpsc::channel();
    let writer = BlockingFlushWriter::new(started_tx);
    let handler = FemtoStreamHandler::with_capacity_timeout(
        writer.clone(),
        DefaultFormatter,
        2,
        Duration::from_secs(1),
    );

    FemtoHandlerTrait::handle(&handler, record("occupies the worker"))
        .expect("the worker should take the first record");
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the worker must reach the writer"),
        1
    );
    // One queue slot is now taken, leaving the other for the flush marker.
    FemtoHandlerTrait::handle(&handler, record("occupies one queue slot"))
        .expect("the free slot should accept the second record");

    // The cap and admission checks see the start, so the marker is admitted
    // with the whole handler budget. Only the acknowledgement sample sees that
    // budget spent.
    let mut clock = ScriptedClock::new(&[0, 0, 30_001]);
    let deadline = clock.deadline();
    let began = Instant::now();
    let confirmed = handler.flush_by_at(deadline, &mut || clock.now());

    assert!(
        !confirmed,
        "a worker blocked inside the writer cannot acknowledge this flush"
    );
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "the acknowledgement must stop at the spent budget, not wait out the handler timeout"
    );
    assert_eq!(
        clock.samples(),
        3,
        "the cap, queue admission and acknowledgement must each sample the clock"
    );

    writer.release_all();
    handler.close();
}

/// A flush whose marker never enters a saturated queue must keep its due drop
/// count for the next warning, rather than swallowing it with the failed flush.
#[test]
fn failed_flush_enqueue_reports_its_drops_on_the_next_warning() {
    install_test_logger();
    let clock = Arc::new(WarningClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(5), clock.clone());

    let (started_tx, started_rx) = mpsc::channel();
    let writer = BlockingFlushWriter::new(started_tx);
    let config = HandlerConfig {
        capacity: 1,
        flush_timeout: Duration::from_millis(100),
        warner,
    };
    let handler = FemtoStreamHandler::with_config(writer.clone(), DefaultFormatter, config);

    FemtoHandlerTrait::handle(&handler, record("occupies the worker"))
        .expect("the worker should take the first record");
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the worker must reach the writer"),
        1
    );
    FemtoHandlerTrait::handle(&handler, record("occupies the only slot"))
        .expect("the free slot should accept one record");

    // The first drop is due and warns at once; the next two stay pending
    // because the interval has not elapsed.
    for _ in 0..3 {
        assert_eq!(
            FemtoHandlerTrait::handle(&handler, record("dropped")),
            Err(HandlerError::QueueFull),
            "a full queue must refuse every further record"
        );
    }

    // Let the interval elapse so the next flush is the vehicle for the count.
    clock.advance(Duration::from_secs(5));
    take_logged_messages();
    assert!(
        !handler.flush(),
        "a full queue must refuse the flush marker"
    );

    // One more dropped record makes the next warning due. The count it carries
    // is the restored one; the record dropped here supplies only one.
    assert_eq!(
        FemtoHandlerTrait::handle(&handler, record("dropped after the flush")),
        Err(HandlerError::QueueFull)
    );
    let warnings = take_logged_messages();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("2 log records dropped")),
        "the restored count must reach the next warning, saw {warnings:?}"
    );

    writer.release_all();
    handler.close();
}
