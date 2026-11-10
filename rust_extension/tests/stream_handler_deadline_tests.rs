//! Deterministic tests for the one deadline `flush()` spends on both waits.
//!
//! These drive a scripted clock through the handler's rate-limiter seam, so the
//! shared budget is exhausted without sleeping. The channel underneath stays
//! real: a gated writer blocks the worker, so the test decides which wait the
//! call reaches, and the elapsed-time assertions only need to show that the
//! call did not fall back on a real timeout.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use _femtologging_rs::rate_limited_warner::{Clock, DEFAULT_WARN_INTERVAL, RateLimitedWarner};
use _femtologging_rs::{
    DefaultFormatter, FemtoLevel, FemtoLogRecord, FemtoStreamHandler, StreamHandlerConfig,
};

#[path = "test_utils/handle_expect.rs"]
mod handle_expect;
use handle_expect::HandleExpect;

/// Configured flush timeout; long enough that resting on it would be obvious.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(4);

/// The same timeout in the millisecond resolution of [`Clock`].
const FLUSH_TIMEOUT_MS: u64 = 4_000;

/// Upper bound on a call that must return without waiting on real time.
///
/// A fresh timeout would take seconds, so any value well below the configured
/// timeout distinguishes the deterministic path from a regression without
/// measuring scheduler jitter.
const PROMPT: Duration = Duration::from_secs(1);

/// Upper bound on how long a test writer waits for its release signal.
const RELEASE_WAIT: Duration = Duration::from_secs(10);

/// Clock whose readings are scripted in advance.
///
/// The handler shares its rate limiter's clock, so scripting that clock drives
/// the flush deadline. Readings are consumed in order and the last one repeats
/// once the script runs dry: sampling more often than expected must not hand
/// back a budget the test meant to spend.
struct ScriptedClock {
    /// Readings still to hand out, and the last one already handed out.
    state: Mutex<ScriptedState>,
}

/// Scripted readings plus the last reading reported.
struct ScriptedState {
    /// Readings not yet reported, consumed in order.
    remaining: VecDeque<u64>,
    /// Most recent reading, repeated once `remaining` is empty.
    last: u64,
}

impl ScriptedClock {
    /// Wrap a clock that reports `readings` in order behind the shared trait.
    ///
    /// The name avoids `new`, which Clippy reserves for constructors returning
    /// `Self`; this constructor erases the type to the handler's seam.
    fn scripted(readings: impl IntoIterator<Item = u64>) -> Arc<dyn Clock> {
        Arc::new(Self {
            state: Mutex::new(ScriptedState {
                remaining: readings.into_iter().collect(),
                last: 0,
            }),
        })
    }
}

impl Clock for ScriptedClock {
    fn now_millis(&self) -> u64 {
        let mut state = self.state.lock().expect("scripted clock lock");
        if let Some(reading) = state.remaining.pop_front() {
            state.last = reading;
        }
        state.last
    }
}

/// Writer whose first write blocks until the test releases it.
struct GatedWriter {
    /// Signals that the worker has reached the first write.
    entered: Sender<()>,
    /// The first write waits for this channel; later writes return at once.
    release: Receiver<()>,
    /// Set once the first write has been released.
    released: bool,
}

impl Write for GatedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.released {
            return Ok(buf.len());
        }
        let _ = self.entered.send(());
        let _ = self.release.recv_timeout(RELEASE_WAIT);
        self.released = true;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Build a one-slot handler whose flush deadline reads `readings` from its clock.
fn scripted_handler(gated: GatedWriter, readings: [u64; 3]) -> FemtoStreamHandler {
    let warner =
        RateLimitedWarner::with_clock(DEFAULT_WARN_INTERVAL, ScriptedClock::scripted(readings));
    let config = StreamHandlerConfig::default()
        .with_capacity(1)
        .with_timeout(FLUSH_TIMEOUT)
        .with_warner(warner);
    FemtoStreamHandler::with_test_config(gated, DefaultFormatter, config)
}

/// Build a gated writer plus the signals its test drives.
fn gated_writer() -> (GatedWriter, Receiver<()>, Sender<()>) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    (
        GatedWriter {
            entered: entered_tx,
            release: release_rx,
            released: false,
        },
        entered_rx,
        release_tx,
    )
}

/// Block the worker inside its first record, before any flush can acknowledge.
fn block_worker(handler: &FemtoStreamHandler, entered: &Receiver<()>) {
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "blocking"));
    entered
        .recv_timeout(RELEASE_WAIT)
        .expect("worker should block in its first write");
}

/// The acknowledgement must be given only the shared deadline's remainder.
///
/// The clock reports no progress while the command is queued, then jumps past
/// the deadline. Since the deadline is only ever derived once, the
/// acknowledgement wait must see an exhausted budget and fail at once; a fresh
/// timeout per wait would instead spend the whole configured timeout here.
#[test]
fn stream_handler_flush_spends_the_shared_deadline_on_the_acknowledgement() {
    let (writer, entered, release) = gated_writer();
    let handler = scripted_handler(writer, [0, 0, FLUSH_TIMEOUT_MS + 1]);
    block_worker(&handler, &entered);

    let started = Instant::now();
    assert!(
        !handler.flush(),
        "an acknowledgement that never arrives must fail the flush"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < PROMPT,
        "flush() waited {elapsed:?}; the acknowledgement was given a fresh timeout \
         instead of the shared deadline's remainder"
    );

    release.send(()).expect("release the blocked writer");
    handler.close();
}

/// Queueing the command must be bounded by the same shared deadline.
///
/// The worker holds the only queue slot full, and the clock is already past the
/// deadline when the queueing wait starts, so the call must fail rather than
/// wait out a fresh timeout for space that never comes.
#[test]
fn stream_handler_flush_stops_queueing_once_the_shared_deadline_expires() {
    let (writer, entered, release) = gated_writer();
    let handler = scripted_handler(writer, [0, FLUSH_TIMEOUT_MS + 1, FLUSH_TIMEOUT_MS + 1]);
    block_worker(&handler, &entered);
    // Fill the only queue slot while the worker is blocked, so the flush
    // command cannot be queued until the worker drains it.
    handler.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, "queued"));

    let started = Instant::now();
    assert!(
        !handler.flush(),
        "a flush that cannot be queued within the deadline must fail"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < PROMPT,
        "flush() waited {elapsed:?}; queueing was not bounded by the shared deadline"
    );

    release.send(()).expect("release the blocked writer");
    handler.close();
}
