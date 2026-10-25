//! Worker thread driving socket I/O.
//!
//! Shutdown is carried on a [`ShutdownSignal`] rather than on the command
//! channel, because the command channel is where the backlog lives. A worker
//! that could only hear a shutdown by reading its queue would have to write
//! the record in hand first, and against a collector that accepts connections
//! and then stalls, that record costs the configured connect and write
//! timeouts before anyone learns anything.

use std::{
    io,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use log::warn;

use crate::{
    handler::HandlerError, log_record::FemtoLogRecord, rate_limited_warner::RateLimitedWarner,
    shutdown_signal::ShutdownSignal,
};

use super::{
    backoff::BackoffState,
    config::SocketHandlerConfig,
    serialize::{frame_payload, serialize_record},
    transport::{ActiveConnection, connect_transport},
};

/// Commands processed by the worker thread.
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "Record variant is the hot path; wrapping in Box would add indirection for no benefit"
)]
pub enum SocketCommand {
    /// Carry one record for serialization and socket delivery.
    Record(FemtoLogRecord),
    /// Flush the active connection and acknowledge completion.
    Flush(Sender<()>),
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
                warn_drops(&self.warner, |count| {
                    warn!(
                        "FemtoSocketHandler dropped {count} records due to serialization failures"
                    );
                });
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
            warn_drops(&self.warner, |count| {
                warn!("FemtoSocketHandler failed to connect: {err}; dropped {count} records");
            });
        }
        self.sleep_if_backing_off(now);
    }

    /// Drop a failed connection; a write error clears it before backoff begins.
    fn handle_write_error(&mut self, err: io::Error, now: Instant) {
        warn!("FemtoSocketHandler write failed: {err}");
        self.connection = None;
        warn_drops(&self.warner, |count| {
            warn!("FemtoSocketHandler dropped {count} records due to write errors");
        });
        self.sleep_if_backing_off(now);
    }

    /// Count a record refused by an exhausted shutdown budget.
    fn warn_shutdown_drops(&self) {
        warn_drops(&self.warner, |count| {
            warn!("FemtoSocketHandler dropped {count} records during shutdown");
        });
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
        // the configuration's full write timeout.
        let Some(timeout) = self.signal.clamp(self.config.write_timeout) else {
            self.warn_shutdown_drops();
            return;
        };
        if let Some(conn) = self.connection.as_mut() {
            let write_result = conn
                .set_write_timeout(timeout)
                .and_then(|_| conn.write_all(frame))
                .and_then(|_| conn.flush());
            match write_result {
                Ok(()) => {
                    self.backoff.record_success(now);
                    self.backoff.reset_after_idle(now);
                }
                Err(err) => self.handle_write_error(err, now),
            }
        }
    }

    /// Flush and acknowledge; even a disconnected worker replies to the request.
    fn handle_flush_command(&mut self, ack: Sender<()>) {
        let success = self.flush_connection();
        let _ = ack.send(());
        if !success {
            warn!("FemtoSocketHandler flush requested without active connection");
        }
    }

    /// Flush the active connection within the remaining write budget.
    ///
    /// Example: no active connection returns `false` without network I/O.
    fn flush_connection(&mut self) -> bool {
        match self.connection.as_mut() {
            Some(conn) => {
                let result = self.signal.clamp(self.config.write_timeout).map_or_else(
                    || Ok(()),
                    |timeout| conn.set_write_timeout(timeout).and_then(|_| conn.flush()),
                );
                if result.is_err() {
                    self.connection = None;
                }
                result.is_ok()
            }
            None => false,
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
        let mut discarded = 0_u64;
        let mut shutdown_acks = Vec::new();
        loop {
            if self.signal.is_aborted() {
                discarded += discard_rest(rx, &mut shutdown_acks);
                break;
            }
            match rx.try_recv() {
                Ok(SocketCommand::Record(record)) => self.handle_record_command(record),
                Ok(SocketCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(SocketCommand::Shutdown(ack)) => shutdown_acks.push(ack),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        warn_discarded(discarded);
        self.warner.flush(|count| {
            warn!("FemtoSocketHandler dropped {count} records during shutdown");
        });
        self.flush_silently();
        // Keep every shutdown acknowledgement behind the last request and
        // cleanup. The final send is the worker's final action.
        for ack in shutdown_acks {
            let _ = ack.send(());
        }
        if let Some(ack) = shutdown_ack {
            let _ = ack.send(());
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
                Ok(SocketCommand::Flush(ack)) => self.handle_flush_command(ack),
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

/// Throw away everything left in the queue, without touching the network.
///
/// Returns how many records were discarded. Flush senders receive their
/// acknowledgement immediately; shutdown senders are saved for the worker's
/// final action, after it has finished all remaining cleanup.
fn discard_rest(rx: &Receiver<SocketCommand>, shutdown_acks: &mut Vec<Sender<()>>) -> u64 {
    let mut discarded = 0_u64;
    loop {
        match rx.try_recv() {
            Ok(SocketCommand::Record(_)) => discarded += 1,
            Ok(SocketCommand::Flush(ack)) => {
                let _ = ack.send(());
            }
            Ok(SocketCommand::Shutdown(ack)) => shutdown_acks.push(ack),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return discarded,
        }
    }
}

/// Say how many records the shutdown deadline cost, once.
///
/// Deliberately not rate-limited through the warner: this fires at most once
/// per handler, and a count of the records a shutdown abandoned is the kind of
/// thing a caller needs to see whether or not a warning interval has elapsed.
fn warn_discarded(discarded: u64) {
    if discarded == 0 {
        return;
    }
    warn!("FemtoSocketHandler discarded {discarded} queued records at its shutdown deadline");
}

/// Build a framed record; payloads within `max_frame_size` are writable.
fn prepare_frame(record: &FemtoLogRecord, config: &SocketHandlerConfig) -> io::Result<Vec<u8>> {
    serialize_record(record).and_then(|payload| {
        frame_payload(&payload, config.max_frame_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))
    })
}

/// Count a failed record; repeated failures warn when the rate limit expires.
fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
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
            warner.record_drop();
            warner.warn_if_due(|count| {
                warn!("FemtoSocketHandler queue full; dropped {count} records");
            });
            Err(HandlerError::QueueFull)
        }
        Err(TrySendError::Disconnected(_)) => {
            warner.record_drop();
            warner.warn_if_due(|count| {
                warn!("FemtoSocketHandler disconnected; dropped {count} records");
            });
            Err(HandlerError::Closed)
        }
    }
}

/// Send a flush command and wait one budget for its acknowledgement.
///
/// The whole wait, not each half of it: the send and the receive share a
/// single deadline, so a send that spends the budget leaves the receive
/// nothing rather than starting a fresh timeout of its own.
pub fn flush_queue(tx: &Sender<SocketCommand>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(SocketCommand::Flush(ack_tx), timeout)
        .is_err()
    {
        return false;
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    ack_rx.recv_timeout(remaining).is_ok()
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
