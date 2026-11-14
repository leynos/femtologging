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
