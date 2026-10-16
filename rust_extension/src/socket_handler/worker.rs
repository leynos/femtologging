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
    Record(FemtoLogRecord),
    Flush(Sender<()>),
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

fn worker_loop(
    rx: Receiver<SocketCommand>,
    config: SocketHandlerConfig,
    signal: Arc<ShutdownSignal>,
) {
    Worker::new(config, signal).run(rx);
}

struct Worker {
    config: SocketHandlerConfig,
    connection: Option<ActiveConnection>,
    backoff: BackoffState,
    warner: RateLimitedWarner,
    signal: Arc<ShutdownSignal>,
}

impl Worker {
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

    /// Wait out a backoff delay, or wake when a shutdown arrives.
    ///
    /// The replacement for the bare sleep this used to be. A delay that
    /// outlives the deadline it was protecting is time nobody is willing to
    /// spend, and a close that lands during one is observed here rather than
    /// after it.
    fn sleep_if_backing_off(&mut self, now: Instant) {
        if let Some(delay) = self.backoff.next_sleep(now) {
            let _ = self.signal.wait(delay);
        }
    }

    fn handle_connect_error(&mut self, err: io::Error, now: Instant) {
        warn_drops(&self.warner, |count| {
            warn!("FemtoSocketHandler failed to connect: {err}; dropped {count} records");
        });
        self.sleep_if_backing_off(now);
    }

    fn handle_write_error(&mut self, err: io::Error, now: Instant) {
        warn!("FemtoSocketHandler write failed: {err}");
        self.connection = None;
        warn_drops(&self.warner, |count| {
            warn!("FemtoSocketHandler dropped {count} records due to write errors");
        });
        self.sleep_if_backing_off(now);
    }

    fn ensure_connection(&mut self, now: Instant) -> bool {
        if self.connection.is_some() {
            return true;
        }
        let Some(budget) = self.signal.clamp(self.config.connect_timeout) else {
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

    fn send_frame_to_connection(&mut self, frame: &[u8], now: Instant) {
        if !self.ensure_connection(now) {
            return;
        }
        // Re-clamped here because opening the connection may itself have spent
        // the budget: a write started after a slow connect must not be handed
        // the configuration's full write timeout.
        let Some(timeout) = self.signal.clamp(self.config.write_timeout) else {
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

    fn handle_flush_command(&mut self, ack: Sender<()>) {
        let success = self.flush_connection();
        let _ = ack.send(());
        if !success {
            warn!("FemtoSocketHandler flush requested without active connection");
        }
    }

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

    /// Flush the connection for the last time, ignoring the outcome.
    ///
    /// Runs before the shutdown acknowledgement so the acknowledgement is the
    /// worker's final act. A caller released by it is entitled to conclude the
    /// handler is done, and it would not be if a flush followed.
    fn flush_silently(&mut self) {
        if let Some(conn) = self.connection.as_mut() {
            let _ = self.signal.clamp(self.config.write_timeout).map(|timeout| {
                let _ = conn.set_write_timeout(timeout);
                let _ = conn.flush();
            });
        }
    }

    /// Attempt each queued command once, then acknowledge the shutdown.
    ///
    /// No retries: `handle_*_error` only sleeps when the signal is running, and
    /// by the time a drain is under way it is not. Nothing is attempted once
    /// the deadline has passed, and the rest of the queue is discarded rather
    /// than written, because a queue that outlasts its deadline is a queue
    /// nobody is still waiting for.
    fn drain_pending(&mut self, rx: &Receiver<SocketCommand>, shutdown_ack: Option<Sender<()>>) {
        let mut discarded = 0_u64;
        loop {
            if self.signal.is_aborted() {
                discarded += discard_rest(rx);
                break;
            }
            match rx.try_recv() {
                Ok(SocketCommand::Record(record)) => self.handle_record_command(record),
                Ok(SocketCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(SocketCommand::Shutdown(ack)) => {
                    let _ = ack.send(());
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        warn_discarded(discarded);
        self.flush_silently();
        // Last, so a handler waiting on the acknowledgement knows the queue
        // behind it has been dealt with.
        if let Some(ack) = shutdown_ack {
            let _ = ack.send(());
        }
    }

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
/// Returns how many records were discarded. Flush and shutdown senders still
/// receive their acknowledgement: their callers are blocked on a reply, and a
/// worker on its way out is the last thing that will send one.
fn discard_rest(rx: &Receiver<SocketCommand>) -> u64 {
    let mut discarded = 0_u64;
    loop {
        match rx.try_recv() {
            Ok(SocketCommand::Record(_)) => discarded += 1,
            Ok(SocketCommand::Flush(ack)) => {
                let _ = ack.send(());
            }
            Ok(SocketCommand::Shutdown(ack)) => {
                let _ = ack.send(());
            }
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

fn prepare_frame(record: &FemtoLogRecord, config: &SocketHandlerConfig) -> io::Result<Vec<u8>> {
    serialize_record(record).and_then(|payload| {
        frame_payload(&payload, config.max_frame_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))
    })
}

fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
}

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
