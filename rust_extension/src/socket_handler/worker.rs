//! Worker thread driving socket I/O.

use std::{
    io, thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError, TrySendError};
use log::warn;

use crate::{
    handler::HandlerError, log_record::FemtoLogRecord, rate_limited_warner::RateLimitedWarner,
};

use super::{
    backoff::BackoffState,
    config::SocketHandlerConfig,
    serialise::{frame_payload, serialise_record},
    transport::{connect_transport, ActiveConnection},
};

/// Commands processed by the worker thread.
#[derive(Debug)]
pub enum SocketCommand {
    Record(FemtoLogRecord),
    Flush(Sender<()>),
    Shutdown(Sender<()>),
}

pub fn spawn_worker(
    config: SocketHandlerConfig,
) -> (Sender<SocketCommand>, thread::JoinHandle<()>) {
    let (tx, rx) = bounded(config.capacity);
    let handle = thread::spawn(move || Worker::new(config).run(rx));
    (tx, handle)
}

struct Worker {
    config: SocketHandlerConfig,
    connection: Option<ActiveConnection>,
    backoff: BackoffState,
    warner: RateLimitedWarner,
}

impl Worker {
    fn new(config: SocketHandlerConfig) -> Self {
        let warn_interval = config.warn_interval;
        let backoff_policy = config.backoff.clone();
        Self {
            backoff: BackoffState::new(backoff_policy),
            warner: RateLimitedWarner::new(warn_interval),
            connection: None,
            config,
        }
    }

    fn run(mut self, rx: Receiver<SocketCommand>) {
        loop {
            match rx.recv() {
                Ok(cmd) => {
                    if self.handle_command(cmd, &rx) {
                        break;
                    }
                }
                Err(_) => {
                    self.drain_pending(&rx);
                    break;
                }
            }
        }
        self.flush_silently();
    }

    fn handle_command(&mut self, cmd: SocketCommand, rx: &Receiver<SocketCommand>) -> bool {
        match cmd {
            SocketCommand::Record(record) => {
                self.process_record(record);
                false
            }
            SocketCommand::Flush(ack) => {
                let success = self.flush_connection();
                let _ = ack.send(());
                if !success {
                    warn!("FemtoSocketHandler flush requested without active connection");
                }
                false
            }
            SocketCommand::Shutdown(ack) => {
                self.drain_pending(rx);
                let success = self.flush_connection();
                let _ = ack.send(());
                if !success {
                    warn!("FemtoSocketHandler flush requested without active connection");
                }
                true
            }
        }
    }

    fn process_record(&mut self, record: FemtoLogRecord) {
        let frame = match self.serialise_frame(&record) {
            Ok(frame) => frame,
            Err(err) => {
                warn!("FemtoSocketHandler serialisation error: {err}");
                self.warn_drops(|count| {
                    warn!(
                        "FemtoSocketHandler dropped {count} records due to serialisation failures"
                    );
                });
                return;
            }
        };
        let now = Instant::now();
        self.send_frame(&frame, now);
    }

    fn serialise_frame(&self, record: &FemtoLogRecord) -> io::Result<Vec<u8>> {
        serialise_record(record).and_then(|payload| {
            frame_payload(&payload, self.config.max_frame_size)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))
        })
    }

    fn send_frame(&mut self, frame: &[u8], now: Instant) {
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
                Err(err) => self.handle_write_failure(err, now),
            }
        }
    }

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
                self.handle_connect_failure(err, now);
                false
            }
        }
    }

    fn handle_connect_failure(&mut self, err: io::Error, now: Instant) {
        self.warn_drops(|count| {
            warn!("FemtoSocketHandler failed to connect: {err}; dropped {count} records");
        });
        if let Some(delay) = self.backoff.next_sleep(now) {
            thread::sleep(delay);
        }
    }

    fn handle_write_failure(&mut self, err: io::Error, now: Instant) {
        warn!("FemtoSocketHandler write failed: {err}");
        self.connection = None;
        self.warn_drops(|count| {
            warn!("FemtoSocketHandler dropped {count} records due to write errors");
        });
        if let Some(delay) = self.backoff.next_sleep(now) {
            thread::sleep(delay);
        }
    }

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

    fn flush_silently(&mut self) {
        if let Some(conn) = self.connection.as_mut() {
            let _ = conn.set_write_timeout(self.config.write_timeout);
            let _ = conn.flush();
        }
    }

    fn drain_pending(&mut self, rx: &Receiver<SocketCommand>) {
        loop {
            match rx.try_recv() {
                Ok(SocketCommand::Record(record)) => self.process_record(record),
                Ok(SocketCommand::Flush(ack)) => {
                    let success = self.flush_connection();
                    let _ = ack.send(());
                    if !success {
                        warn!("FemtoSocketHandler flush requested without active connection");
                    }
                }
                Ok(SocketCommand::Shutdown(ack)) => {
                    let _ = ack.send(());
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn warn_drops(&self, log: impl FnMut(u64)) {
        self.warner.record_drop();
        self.warner.warn_if_due(log);
    }
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
