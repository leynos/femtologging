//! A scripted writer that holds the file-handler worker at a known point.
//!
//! The behavioural tests need the worker to be blocked inside `write` or
//! `flush` at a moment the test chooses. Sleeping would make the assertions
//! timing-dependent, so the writer reports each stage over a channel and then
//! waits on a gate the test opens explicitly.

use super::test_support::impl_unsupported_seek;
use super::*;
use crate::formatter::DefaultFormatter;
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

/// A point at which the scripted writer can hold the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    /// The worker is inside `write`.
    Write,
    /// The worker is inside `flush`.
    Flush,
}

/// What the scripted writer does once its flush gate opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlushBehaviour {
    /// Report success.
    Succeed,
    /// Report an I/O failure.
    Fail,
    /// Panic, simulating a worker that dies mid-flush.
    Panic,
}

/// What the scripted writer does once its write gate opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteBehaviour {
    /// Accept the bytes.
    Succeed,
    /// Panic, simulating a worker that dies mid-write.
    Panic,
}

/// A one-shot gate the test opens to release a blocked worker.
#[derive(Default)]
struct Gate {
    /// Whether the gate has been opened.
    is_open: Mutex<bool>,
    /// Wakes waiters when the gate opens.
    opened: Condvar,
}

impl Gate {
    /// Create a closed gate.
    fn closed() -> Self {
        Self::default()
    }

    /// Create a gate that is already open.
    fn opened() -> Self {
        let gate = Self::default();
        gate.open();
        gate
    }

    /// Block until the test opens the gate.
    fn wait(&self) {
        // Recover from poisoning: the flag is always valid data.
        let mut is_open = self
            .is_open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*is_open {
            is_open = self
                .opened
                .wait(is_open)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Release every waiter and every later arrival.
    fn open(&self) {
        // Recover from poisoning: the flag is always valid data.
        let mut is_open = self
            .is_open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *is_open = true;
        self.opened.notify_all();
    }
}

/// Writer that reports each stage to the test and can block or fail there.
#[derive(Clone)]
pub(super) struct ScriptedWriter {
    /// Gate the worker waits on inside `write`.
    write_gate: Arc<Gate>,
    /// Gate the worker waits on inside `flush`.
    flush_gate: Arc<Gate>,
    /// What `flush` does once its gate opens.
    flush_behaviour: FlushBehaviour,
    /// What `write` does once its gate opens.
    write_behaviour: WriteBehaviour,
    /// Reports the stage the worker has reached.
    entered: mpsc::Sender<Stage>,
}

impl ScriptedWriter {
    /// Build a writer whose gates start in the requested state.
    fn new(
        write_gate: Arc<Gate>,
        flush_gate: Arc<Gate>,
        flush_behaviour: FlushBehaviour,
        write_behaviour: WriteBehaviour,
        entered: mpsc::Sender<Stage>,
    ) -> Self {
        Self {
            write_gate,
            flush_gate,
            flush_behaviour,
            write_behaviour,
            entered,
        }
    }

    /// Build a writer that blocks in `write` and `flush` until released.
    pub(super) fn blocking(entered: mpsc::Sender<Stage>) -> Self {
        Self::new(
            Arc::new(Gate::closed()),
            Arc::new(Gate::closed()),
            FlushBehaviour::Succeed,
            WriteBehaviour::Succeed,
            entered,
        )
    }

    /// Build a writer whose `flush` fails immediately.
    pub(super) fn failing_flush(entered: mpsc::Sender<Stage>) -> Self {
        Self::new(
            Arc::new(Gate::closed()),
            Arc::new(Gate::opened()),
            FlushBehaviour::Fail,
            WriteBehaviour::Succeed,
            entered,
        )
    }

    /// Build a writer whose `flush` panics immediately.
    pub(super) fn panicking_flush(entered: mpsc::Sender<Stage>) -> Self {
        Self::new(
            Arc::new(Gate::closed()),
            Arc::new(Gate::opened()),
            FlushBehaviour::Panic,
            WriteBehaviour::Succeed,
            entered,
        )
    }

    /// Build a writer that blocks in `flush` and panics once released.
    pub(super) fn blocking_then_panicking_flush(entered: mpsc::Sender<Stage>) -> Self {
        Self::new(
            Arc::new(Gate::opened()),
            Arc::new(Gate::closed()),
            FlushBehaviour::Panic,
            WriteBehaviour::Succeed,
            entered,
        )
    }

    /// Build a writer that panics inside `write`.
    pub(super) fn panicking_writes(entered: mpsc::Sender<Stage>) -> Self {
        Self::new(
            Arc::new(Gate::opened()),
            Arc::new(Gate::opened()),
            FlushBehaviour::Succeed,
            WriteBehaviour::Panic,
            entered,
        )
    }

    /// Release both gates so the worker can finish.
    pub(super) fn release(&self) {
        self.write_gate.open();
        self.flush_gate.open();
    }
}

impl Write for ScriptedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // A disconnected receiver only means the test stopped listening.
        let _ = self.entered.send(Stage::Write);
        self.write_gate.wait();
        match self.write_behaviour {
            WriteBehaviour::Succeed => Ok(buf.len()),
            WriteBehaviour::Panic => panic!("simulated worker write panic"),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.entered.send(Stage::Flush);
        self.flush_gate.wait();
        match self.flush_behaviour {
            FlushBehaviour::Succeed => Ok(()),
            FlushBehaviour::Fail => Err(io::Error::other("simulated flush failure")),
            FlushBehaviour::Panic => panic!("simulated worker flush panic"),
        }
    }
}

impl_unsupported_seek!(ScriptedWriter);

/// Build a handler whose worker is driven by `writer`.
pub(super) fn handler_with(writer: ScriptedWriter, policy: OverflowPolicy) -> FemtoFileHandler {
    let mut cfg = TestConfig::new(writer, DefaultFormatter);
    cfg.capacity = 1;
    // Disable periodic flushing so only the explicit and terminal flushes
    // reach the scripted writer.
    cfg.flush_interval = 0;
    cfg.overflow_policy = policy;
    FemtoFileHandler::with_writer_for_test(cfg)
}

/// Build one record for the enqueue paths.
pub(super) fn record(message: &str) -> FemtoLogRecord {
    FemtoLogRecord::new("core", FemtoLevel::Info, message)
}

/// Wait for the worker to reach `stage`.
pub(super) fn expect_stage(entered: &mpsc::Receiver<Stage>, stage: Stage) {
    assert_eq!(
        entered.recv_timeout(Duration::from_secs(2)),
        Ok(stage),
        "the worker never reached {stage:?}",
    );
}
