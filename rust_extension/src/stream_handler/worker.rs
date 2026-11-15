//! The stream worker's command loop and per-command handlers.
//!
//! The worker owns the writer and formatter on its own thread. Each command is
//! handled in queue order: a record is formatted, written, and flushed, and a
//! flush command flushes the writer and returns the exact I/O result to the
//! producer waiting on it.
//!
//! The loop runs inside a completion guard so that the writer and formatter
//! are dropped, and a panicking worker still wakes every closer, before the
//! guard reports completion.

use std::io::{self, Write};
use std::sync::Arc;

use log::warn;

use super::StreamCommand;
use crate::formatter::FemtoFormatter;
use crate::log_record::FemtoLogRecord;
use crate::shutdown::{ShutdownCompletion, run_worker_guarded};
use crate::sync::Receiver;

/// Flushes the writer during worker shutdown and logs, but does not propagate, errors.
fn flush_with_warning<W: Write>(writer: &mut W) {
    if writer.flush().is_err() {
        warn!("FemtoStreamHandler flush error");
    }
}

/// Formats one queued record, writes a newline, and flushes the worker-owned writer.
fn handle_record_command<W, F>(writer: &mut W, formatter: &F, record: FemtoLogRecord)
where
    W: Write,
    F: FemtoFormatter,
{
    let msg = formatter.format(&record);
    if writeln!(writer, "{msg}")
        .and_then(|_| writer.flush())
        .is_err()
    {
        warn!("FemtoStreamHandler write error");
    }
}

/// Flushes the writer and sends the exact I/O result to the waiting producer.
fn handle_flush_command<W: Write>(writer: &mut W, ack: crate::sync::Sender<io::Result<()>>) {
    let flush_result = writer.flush();
    if flush_result.is_err() {
        warn!("FemtoStreamHandler flush error");
    }
    let _ = ack.send(flush_result);
}

/// Owns the writer and formatter on the background thread until the queue closes.
pub(super) fn run_stream_worker<W, F>(
    rx: Receiver<StreamCommand>,
    writer: W,
    formatter: F,
    completion: Arc<ShutdownCompletion>,
) where
    W: Write,
    F: FemtoFormatter,
{
    // The guard is declared before the work moves the writer and formatter in,
    // so those captures drop before completion is reported, including while
    // the worker unwinds. The work closure exists for exactly that lifetime.
    run_worker_guarded(completion, move || {
        let mut writer = writer;
        for cmd in rx {
            match cmd {
                StreamCommand::Record(record) => {
                    handle_record_command(&mut writer, &formatter, record);
                }
                StreamCommand::Flush(ack) => handle_flush_command(&mut writer, ack),
            }
        }
        flush_with_warning(&mut writer);
    });
}

#[cfg(test)]
mod tests {
    //! Tests that the worker's completion signal follows its destructors.

    use std::time::Duration;

    use super::*;
    use crate::formatter::DefaultFormatter;
    use crate::shutdown::ShutdownCompletion;
    use crate::sync::{Receiver, Sender, bounded};

    /// Writer that holds the worker inside its own destructor until released.
    struct BlockingDropWriter {
        /// Signals the test that the destructor has started.
        entered_drop: Sender<()>,
        /// Keeps the destructor running until the test says otherwise.
        release_drop: Receiver<()>,
    }

    impl Write for BlockingDropWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for BlockingDropWriter {
        fn drop(&mut self) {
            let _ = self.entered_drop.send(());
            let _ = self.release_drop.recv();
        }
    }

    /// The completion signal must not arrive while the writer is still alive.
    ///
    /// A caller that acts on the signal is entitled to assume the worker has
    /// released everything it owned, so a `Drop` that runs after the send leaks
    /// an observable ordering the caller cannot rely on.
    #[test]
    fn completion_signal_follows_the_worker_destructors() {
        let (cmd_tx, cmd_rx) = bounded(1);
        let (drop_started_tx, drop_started_rx) = bounded(1);
        let (release_drop_tx, release_drop_rx) = bounded(1);
        let completion = ShutdownCompletion::new();
        let waiter = completion.register();

        let worker_completion = Arc::clone(&completion);
        let worker = std::thread::spawn(move || {
            run_stream_worker(
                cmd_rx,
                BlockingDropWriter {
                    entered_drop: drop_started_tx,
                    release_drop: release_drop_rx,
                },
                DefaultFormatter,
                worker_completion,
            );
        });
        drop(cmd_tx);

        drop_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker should reach its writer's destructor");
        let completed_early = waiter.try_recv().is_ok();
        release_drop_tx
            .send(())
            .expect("the worker should still be in its writer's destructor");
        worker.join().expect("the worker thread should not panic");

        assert!(
            !completed_early,
            "the completion signal arrived before the writer was dropped",
        );
        assert!(
            completion.wait(Duration::from_secs(5)),
            "the worker should signal completion once its writer is released",
        );
    }
}
