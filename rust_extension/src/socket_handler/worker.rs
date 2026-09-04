//! Worker thread driving socket I/O.

use std::{
    io, thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use log::warn;

use crate::{
    handler::HandlerError, log_record::FemtoLogRecord, rate_limited_warner::RateLimitedWarner,
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
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    Record(FemtoLogRecord),
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    Flush(Sender<()>),
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    Shutdown(Sender<()>),
}

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
pub fn spawn_worker(
    config: SocketHandlerConfig,
) -> (Sender<SocketCommand>, thread::JoinHandle<()>) {
    let (tx, rx) = bounded(config.capacity);
    let handle = thread::spawn(move || worker_loop(rx, config));
    (tx, handle)
}

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
fn worker_loop(rx: Receiver<SocketCommand>, config: SocketHandlerConfig) {
    Worker::new(config).run(rx);
}

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
struct Worker {
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    config: SocketHandlerConfig,
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    connection: Option<ActiveConnection>,
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    backoff: BackoffState,
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    warner: RateLimitedWarner,
}

impl Worker {
    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn new(config: SocketHandlerConfig) -> Self {
        let backoff = BackoffState::new(config.backoff.clone());
        let warner = RateLimitedWarner::new(config.warn_interval);
        Self {
            config,
            connection: None,
            backoff,
            warner,
        }
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
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

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn sleep_if_backing_off(&mut self, now: Instant) {
        if let Some(delay) = self.backoff.next_sleep(now) {
            thread::sleep(delay);
        }
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn handle_connect_error(&mut self, err: io::Error, now: Instant) {
        warn_drops(&self.warner, |count| {
            warn!("FemtoSocketHandler failed to connect: {err}; dropped {count} records");
        });
        self.sleep_if_backing_off(now);
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn handle_write_error(&mut self, err: io::Error, now: Instant) {
        warn!("FemtoSocketHandler write failed: {err}");
        self.connection = None;
        warn_drops(&self.warner, |count| {
            warn!("FemtoSocketHandler dropped {count} records due to write errors");
        });
        self.sleep_if_backing_off(now);
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn ensure_connection(&mut self, now: Instant) -> bool {
        if self.connection.is_some() {
            return true;
        }
        match connect_transport(&self.config.transport, self.config.connect_timeout) {
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

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn send_frame_to_connection(&mut self, frame: &[u8], now: Instant) {
        if !self.ensure_connection(now) {
            return;
        }
        if let Some(conn) = self.connection.as_mut() {
            let write_result = conn
                .set_write_timeout(self.config.write_timeout)
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

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn handle_flush_command(&mut self, ack: Sender<()>) {
        let success = self.flush_connection();
        let _ = ack.send(());
        if !success {
            warn!("FemtoSocketHandler flush requested without active connection");
        }
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn flush_connection(&mut self) -> bool {
        match self.connection.as_mut() {
            Some(conn) => {
                let result = conn
                    .set_write_timeout(self.config.write_timeout)
                    .and_then(|_| conn.flush());
                if result.is_err() {
                    self.connection = None;
                }
                result.is_ok()
            }
            None => false,
        }
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn flush_silently(&mut self) {
        if let Some(conn) = self.connection.as_mut() {
            let _ = conn.set_write_timeout(self.config.write_timeout);
            let _ = conn.flush();
        }
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn drain_pending(&mut self, rx: &Receiver<SocketCommand>) {
        loop {
            match rx.try_recv() {
                Ok(SocketCommand::Record(record)) => self.handle_record_command(record),
                Ok(SocketCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(SocketCommand::Shutdown(ack)) => {
                    let _ = ack.send(());
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
    fn run(mut self, rx: Receiver<SocketCommand>) {
        loop {
            match rx.recv() {
                Ok(SocketCommand::Record(record)) => self.handle_record_command(record),
                Ok(SocketCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(SocketCommand::Shutdown(ack)) => {
                    self.drain_pending(&rx);
                    self.handle_flush_command(ack);
                    break;
                }
                Err(_) => {
                    self.drain_pending(&rx);
                    break;
                }
            }
        }
        self.flush_silently();
    }
}

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
fn prepare_frame(record: &FemtoLogRecord, config: &SocketHandlerConfig) -> io::Result<Vec<u8>> {
    serialize_record(record).and_then(|payload| {
        frame_payload(&payload, config.max_frame_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))
    })
}

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
}

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
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

/// Supports socket delivery while keeping connection, retry, and byte-serialisation state on the worker side of the queue.
pub fn flush_queue(tx: &Sender<SocketCommand>, timeout: Duration) -> bool {
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(SocketCommand::Flush(ack_tx), timeout)
        .is_err()
    {
        return false;
    }
    ack_rx.recv_timeout(timeout).is_ok()
}
