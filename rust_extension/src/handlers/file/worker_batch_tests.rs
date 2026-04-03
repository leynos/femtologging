//! Tests for file-worker batch configuration and command draining.

use super::*;
use crate::formatter::DefaultFormatter;
use crate::level::FemtoLevel;
use crossbeam_channel::bounded;
use std::io::{self, Cursor, Seek, SeekFrom, Write};
use std::time::Duration;

#[derive(Default)]
struct RecordingWriter {
    buffer: Cursor<Vec<u8>>,
    flushes: usize,
}

impl Write for RecordingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

impl Seek for RecordingWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.buffer.seek(pos)
    }
}

/// Create a record command with a stable logger and level for assertions.
fn record(message: &str) -> FileCommand {
    FileCommand::Record(Box::new(FemtoLogRecord::new(
        "core",
        FemtoLevel::Info,
        message,
    )))
}

#[test]
/// Reject zero-capacity batch configs at construction time.
fn batch_config_rejects_zero_capacity() {
    assert_eq!(BatchConfig::new(0), Err(BatchConfigError::ZeroCapacity));
}

#[test]
/// Preserve the configured capacity when construction succeeds.
fn batch_config_accepts_positive_capacity() {
    let config = BatchConfig::new(3).expect("positive capacity should succeed");
    assert_eq!(config.capacity(), 3);
}

#[test]
/// Guard the drain helper against zero-capacity callers.
fn recv_batch_rejects_zero_capacity() {
    let (_tx, rx) = bounded(1);
    assert!(matches!(
        recv_batch(&rx, 0),
        Err(RecvBatchError::ZeroCapacity)
    ));
}

#[test]
/// Drain additional queued commands until the requested capacity is reached.
fn recv_batch_collects_up_to_capacity() {
    let (tx, rx) = bounded(4);
    tx.send(record("one")).expect("first record queued");
    tx.send(record("two")).expect("second record queued");
    tx.send(record("three")).expect("third record queued");

    let batch = recv_batch(&rx, 2).expect("batch should be received");
    assert_eq!(batch.len(), 2);
    assert!(matches!(batch[0], FileCommand::Record(_)));
    assert!(matches!(batch[1], FileCommand::Record(_)));

    let remainder = recv_batch(&rx, 2).expect("remaining record should be received");
    assert_eq!(remainder.len(), 1);
}

#[test]
/// Report disconnection when no command can be received.
fn recv_batch_reports_disconnection_when_empty() {
    let (tx, rx) = bounded::<FileCommand>(1);
    drop(tx);
    assert!(matches!(
        recv_batch(&rx, 1),
        Err(RecvBatchError::Disconnected)
    ));
}

#[test]
/// Process batched records in order and forward flush acknowledgements.
fn process_batch_preserves_record_order_and_flush_acknowledges() {
    let mut state = WorkerState::new(RecordingWriter::default(), NoRotation, 8);
    let (ack_tx, ack_rx) = bounded(1);
    let commands = vec![
        record("first"),
        record("second"),
        FileCommand::Flush(ack_tx),
    ];

    state.process_batch(&DefaultFormatter, commands);

    assert_eq!(
        String::from_utf8(state.writer.buffer.into_inner()).expect("writer output must be UTF-8"),
        "core [INFO] first\ncore [INFO] second\n"
    );
    assert_eq!(state.writer.flushes, 1);
    assert!(
        ack_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("ack should be sent within the timeout")
            .is_ok()
    );
}
