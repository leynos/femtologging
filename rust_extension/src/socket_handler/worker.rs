//! Worker thread driving socket I/O.
//!
//! Shutdown is carried on a [`ShutdownSignal`] rather than on the command
//! channel, because the command channel is where the backlog lives. A worker
//! that could only hear a shutdown by reading its queue would have to write
//! the record in hand first, and against a collector that accepts connections
//! and then stalls, that record costs the configured connect and write
//! timeouts before anyone learns anything.

use std::{io, sync::Arc, thread, time::Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use log::warn;

use crate::{
    handler::{HandlerError, report_drops_then_acknowledge},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
    shutdown_signal::ShutdownSignal,
};

use super::{
    backoff::BackoffState,
    config::SocketHandlerConfig,
    frame_write::FrameWriter,
    serialize::{frame_payload, serialize_record},
    transport::{ActiveConnection, connect_transport},
    warning::{flush_drops_naming_cause, warn_drops_naming_cause, warn_pending_drops},
};

#[path = "worker_drain.rs"]
mod drain;

use drain::{discard_rest, warn_discarded};

/// Commands processed by the worker thread.
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "Record variant is the hot path; wrapping in Box would add indirection for no benefit"
)]
pub enum SocketCommand {
    /// Carry one record for serialization and socket delivery.
    Record(FemtoLogRecord),
    /// Flush the active connection, then acknowledge with the flush outcome
    /// and any drop count the caller took from the warner.
    ///
    /// The outcome travels back so a caller waiting on this marker can tell a
    /// confirmed flush from a connection that failed. Acknowledging both as
    /// success would let a barrier flush report records persisted that the
    /// socket rejected.
    Flush(Sender<io::Result<()>>, Option<u64>),
    /// Begin shutdown and acknowledge after worker cleanup.
    Shutdown(Sender<()>),
}

/// Spawn a socket worker and the signal its handler shares with it.
pub fn spawn_worker(
    config: SocketHandlerConfig,
    signal: Arc<ShutdownSignal>,
) -> (Sender<SocketCommand>, thread::JoinHandle<()>) {
    let (tx, rx) = bounded(config.capacity);
    let handle = thread::spawn(move || worker_loop(rx, config, signal));
    (tx, handle)
}

/// Run the worker loop; the spawned thread supplies its receiver and signal.
fn worker_loop(
    rx: Receiver<SocketCommand>,
    config: SocketHandlerConfig,
    signal: Arc<ShutdownSignal>,
) {
    Worker::new(config, signal).run(rx);
}

/// Owns socket connection state and retry policy for one worker thread.
struct Worker {
    /// Transport, timeout and framing settings for records.
    config: SocketHandlerConfig,
    /// Reused connection, if the latest connect attempt succeeded.
    connection: Option<ActiveConnection>,
    /// Retry delays accumulated across connection and write failures.
    backoff: BackoffState,
    /// Counts discarded records and limits warning frequency.
    warner: RateLimitedWarner,
    /// Lets connection, write and backoff operations observe shutdown.
    signal: Arc<ShutdownSignal>,
}

impl Worker {
    /// Create a worker; it starts disconnected with fresh retry state.
    fn new(config: SocketHandlerConfig, signal: Arc<ShutdownSignal>) -> Self {
        let backoff = BackoffState::new(config.backoff.clone());
        let warner = RateLimitedWarner::new(config.warn_interval);
        Self {
            config,
            connection: None,
            backoff,
            warner,
            signal,
        }
    }

    /// Send one record; an oversized frame is counted as dropped without a write.
    fn handle_record_command(&mut self, record: FemtoLogRecord) {
        let frame = match prepare_frame(&record, &self.config) {
            Ok(frame) => frame,
            Err(err) => {
                warn!("FemtoSocketHandler serialization error: {err}");
                warn_drops_naming_cause(&self.warner, "serialization failure");
                return;
            }
        };
        let now = Instant::now();
        self.send_frame_to_connection(&frame, now);
    }

    /// Wait out backoff; close wakes the wait before its delay expires.
    fn sleep_if_backing_off(&mut self, now: Instant) {
        if let Some(delay) = self.backoff.next_sleep(now) {
            let _ = self.signal.wait(delay);
        }
    }

    /// Count a failed connection; close wakes the following backoff wait.
    fn handle_connect_error(&mut self, err: io::Error, now: Instant) {
        if self.signal.is_shutting_down() {
            self.warn_shutdown_drops();
            warn!("FemtoSocketHandler failed to connect during shutdown: {err}");
        } else {
            warn_drops_naming_cause(&self.warner, &format!("connect failure: {err}"));
        }
        self.sleep_if_backing_off(now);
    }

    /// Drop a failed connection; a write error clears it before backoff begins.
    fn handle_write_error(&mut self, err: io::Error, now: Instant) {
        warn!("FemtoSocketHandler write failed: {err}");
        self.connection = None;
        warn_drops_naming_cause(&self.warner, "write error");
        self.sleep_if_backing_off(now);
    }

    /// Count a record refused by an exhausted shutdown budget.
    ///
    /// This is the last chance to report the refusal, so the count is flushed
    /// without waiting for the warning interval.
    fn warn_shutdown_drops(&self) {
        self.warner.record_drop();
        flush_drops_naming_cause(&self.warner, "abandoned retry during shutdown");
    }

    /// Reuse or connect; an expired shutdown budget returns `false` without I/O.
    fn ensure_connection(&mut self, now: Instant) -> bool {
        if self.connection.is_some() {
            return true;
        }
        let Some(budget) = self.signal.clamp(self.config.connect_timeout) else {
            self.warn_shutdown_drops();
            return false;
        };
        match connect_transport(&self.config.transport, budget, &self.signal) {
            Ok(mut conn) => {
                if let Err(err) = conn.set_write_timeout(self.config.write_timeout) {
                    warn!("FemtoSocketHandler failed to set write timeout: {err}");
                }
                self.backoff.record_success(now);
                self.connection = Some(conn);
                true
            }
            Err(err) => {
                self.handle_connect_error(err, now);
                false
            }
        }
    }

    /// Send a frame; success resets the connection backoff state.
    fn send_frame_to_connection(&mut self, frame: &[u8], now: Instant) {
        if !self.ensure_connection(now) {
            return;
        }
        // Re-clamped here because opening the connection may itself have spent
        // the budget: a write started after a slow connect must not be handed
        // the configuration's full write timeout on the shutdown deadline.
        if self.signal.clamp(self.config.write_timeout).is_none() {
            self.warn_shutdown_drops();
            return;
        }
        if let Some(conn) = self.connection.as_mut() {
            // `FrameWriter` re-clamps between calls; see its own docs for why a
            // `write_all` here could outlast the deadline. The frame's clock
            // starts now rather than at `now`, because a connect that took time
            // was already bounded by the connect timeout, and charging one
            // against the other would fail writes that never blocked at all.
            let mut writer = FrameWriter {
                connection: conn,
                write_timeout: self.config.write_timeout,
                signal: &self.signal,
                started: Instant::now(),
            };
            match writer.write_frame(frame) {
                Ok(()) => {
                    self.backoff.record_success(now);
                    self.backoff.reset_after_idle(now);
                }
                Err(err) => self.handle_write_error(err, now),
            }
        }
    }

    /// Flush and acknowledge; even a disconnected worker replies to the request.
    ///
    /// The drop count the handler moved here is reported before the
    /// acknowledgement is sent, so a caller that observes the flush succeed
    /// cannot do so before the report it carried has been emitted. The
    /// acknowledgement itself carries the flush outcome: an I/O failure is
    /// reported to the caller rather than acknowledged as a successful flush.
    /// The connection warning stays after the acknowledgement because it
    /// describes the flush that just happened rather than reporting drops the
    /// caller handed over.
    fn handle_flush_command(&mut self, ack: Sender<io::Result<()>>, pending_drops: Option<u64>) {
        let had_connection = self.connection.is_some();
        let flush_result = self.flush_connection();
        let flush_failed = flush_result.is_err();
        report_drops_then_acknowledge(pending_drops, warn_pending_drops, || {
            // Ignore send error: a caller that stopped waiting has nothing
            // left to learn from the outcome.
            let _ = ack.send(flush_result);
        });
        if flush_failed {
            warn!("FemtoSocketHandler flush failed; the connection was dropped");
        } else if !had_connection {
            warn!("FemtoSocketHandler flush requested without active connection");
        }
    }

    /// Flush the active connection within the remaining write budget.
    ///
    /// A handler with no active connection has nothing buffered to flush, so
    /// it reports success without network I/O; the caller still warns that the
    /// flush had no connection to confirm.
    fn flush_connection(&mut self) -> io::Result<()> {
        match self.connection.as_mut() {
            Some(conn) => {
                let result = self.signal.clamp(self.config.write_timeout).map_or_else(
                    || Ok(()),
                    |timeout| conn.set_write_timeout(timeout).and_then(|_| conn.flush()),
                );
                if result.is_err() {
                    self.connection = None;
                }
                result
            }
            None => Ok(()),
        }
    }

    /// Flush the connection before acknowledging shutdown, ignoring the outcome.
    fn flush_silently(&mut self) {
        if let Some(conn) = self.connection.as_mut() {
            let _ = self.signal.clamp(self.config.write_timeout).map(|timeout| {
                let _ = conn.set_write_timeout(timeout);
                let _ = conn.flush();
            });
        }
    }

    /// Drain once per record; discard the queue when the shutdown deadline expires.
    fn drain_pending(&mut self, rx: &Receiver<SocketCommand>, shutdown_ack: Option<Sender<()>>) {
        let (discarded, shutdown_acks) = self.discard_queued_commands(rx);
        warn_discarded(discarded);
        self.warner.flush(warn_pending_drops);
        self.flush_silently();
        // Keep every shutdown acknowledgement behind the last request and
        // cleanup. The final send is the worker's final action.
        crate::shutdown_ack::acknowledge_shutdowns(shutdown_acks, shutdown_ack);
    }

    /// Attempt each queued command once, stopping at abort or on an empty queue.
    fn discard_queued_commands(&mut self, rx: &Receiver<SocketCommand>) -> (u64, Vec<Sender<()>>) {
        let mut discarded = 0_u64;
        let mut shutdown_acks = Vec::new();
        while !self.signal.is_aborted() {
            match rx.try_recv() {
                Ok(command) => self.apply_drained_command(command, &mut shutdown_acks),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => {
                    return (discarded, shutdown_acks);
                }
            }
        }
        discarded += discard_rest(rx, &mut shutdown_acks);
        (discarded, shutdown_acks)
    }

    /// Apply one command that was still queued when the shutdown started.
    fn apply_drained_command(
        &mut self,
        command: SocketCommand,
        shutdown_acks: &mut Vec<Sender<()>>,
    ) {
        match command {
            SocketCommand::Record(record) => self.handle_record_command(record),
            SocketCommand::Flush(ack, pending_drops) => {
                self.handle_flush_command(ack, pending_drops);
            }
            SocketCommand::Shutdown(ack) => shutdown_acks.push(ack),
        }
    }

    /// Consume commands; shutdown starts a bounded drain before its acknowledgement.
    fn run(mut self, rx: Receiver<SocketCommand>) {
        loop {
            // Checked before the blocking receive as well as on the command,
            // because the shutdown is carried on the signal and the command
            // channel is only the second way the worker hears about it. A
            // handler that could not queue its shutdown command has still
            // raised the signal, and the worker must stop for that alone
            // rather than wait for a command that may never arrive.
            if self.signal.is_shutting_down() {
                self.drain_pending(&rx, None);
                return;
            }
            match rx.recv() {
                Ok(SocketCommand::Record(record)) => self.handle_record_command(record),
                Ok(SocketCommand::Flush(ack, pending_drops)) => {
                    self.handle_flush_command(ack, pending_drops);
                }
                Ok(SocketCommand::Shutdown(ack)) => {
                    self.drain_pending(&rx, Some(ack));
                    return;
                }
                Err(_) => {
                    self.drain_pending(&rx, None);
                    return;
                }
            }
        }
    }
}

/// Build a framed record; payloads within `max_frame_size` are writable.
fn prepare_frame(record: &FemtoLogRecord, config: &SocketHandlerConfig) -> io::Result<Vec<u8>> {
    serialize_record(record).and_then(|payload| {
        frame_payload(&payload, config.max_frame_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))
    })
}

/// Enqueue one record; a full queue returns `HandlerError::QueueFull` and counts a drop.
pub fn enqueue_record(
    tx: &Sender<SocketCommand>,
    record: FemtoLogRecord,
    warner: &RateLimitedWarner,
) -> Result<(), HandlerError> {
    match tx.try_send(SocketCommand::Record(record)) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            warn_drops_naming_cause(warner, "queue full");
            Err(HandlerError::QueueFull)
        }
        Err(TrySendError::Disconnected(_)) => {
            warn_drops_naming_cause(warner, "worker disconnected");
            Err(HandlerError::Closed)
        }
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
