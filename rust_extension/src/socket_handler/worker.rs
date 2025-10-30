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
    let handle = thread::spawn(move || worker_loop(rx, config));
    (tx, handle)
}

fn worker_loop(rx: Receiver<SocketCommand>, config: SocketHandlerConfig) {
    let mut connection: Option<ActiveConnection> = None;
    let mut backoff = BackoffState::new(config.backoff.clone());
    let warner = RateLimitedWarner::new(config.warn_interval);
    loop {
        match rx.recv() {
            Ok(SocketCommand::Record(record)) => {
                handle_record_command(record, &config, &mut connection, &mut backoff, &warner);
            }
            Ok(SocketCommand::Flush(ack)) => {
                handle_flush_command(ack, &config, &mut connection);
            }
            Ok(SocketCommand::Shutdown(ack)) => {
                drain_pending(&rx, &config, &mut connection, &mut backoff, &warner);
                handle_flush_command(ack, &config, &mut connection);
                break;
            }
            Err(_) => {
                drain_pending(&rx, &config, &mut connection, &mut backoff, &warner);
                break;
            }
        }
    }
    flush_silently(&mut connection, &config);
}

fn handle_record_command(
    record: FemtoLogRecord,
    config: &SocketHandlerConfig,
    connection: &mut Option<ActiveConnection>,
    backoff: &mut BackoffState,
    warner: &RateLimitedWarner,
) {
    let frame = match prepare_frame(&record, config) {
        Ok(frame) => frame,
        Err(err) => {
            warn!("FemtoSocketHandler serialisation error: {err}");
            warn_drops(warner, |count| {
                warn!("FemtoSocketHandler dropped {count} records due to serialisation failures");
            });
            return;
        }
    };
    let now = Instant::now();
    send_frame_to_connection(&frame, now, config, connection, backoff, warner);
}

fn prepare_frame(record: &FemtoLogRecord, config: &SocketHandlerConfig) -> io::Result<Vec<u8>> {
    serialise_record(record).and_then(|payload| {
        frame_payload(&payload, config.max_frame_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))
    })
}

fn ensure_connection(
    now: Instant,
    config: &SocketHandlerConfig,
    connection: &mut Option<ActiveConnection>,
    backoff: &mut BackoffState,
    warner: &RateLimitedWarner,
) -> bool {
    if connection.is_some() {
        return true;
    }
    match connect_transport(&config.transport, config.connect_timeout) {
        Ok(mut conn) => {
            if let Err(err) = conn.set_write_timeout(config.write_timeout) {
                warn!("FemtoSocketHandler failed to set write timeout: {err}");
            }
            backoff.record_success(now);
            *connection = Some(conn);
            true
        }
        Err(err) => {
            warn_drops(warner, |count| {
                warn!("FemtoSocketHandler failed to connect: {err}; dropped {count} records");
            });
            if let Some(delay) = backoff.next_sleep(now) {
                thread::sleep(delay);
            }
            false
        }
    }
}

fn send_frame_to_connection(
    frame: &[u8],
    now: Instant,
    config: &SocketHandlerConfig,
    connection: &mut Option<ActiveConnection>,
    backoff: &mut BackoffState,
    warner: &RateLimitedWarner,
) {
    if !ensure_connection(now, config, connection, backoff, warner) {
        return;
    }
    if let Some(conn) = connection.as_mut() {
        let write_result = conn
            .set_write_timeout(config.write_timeout)
            .and_then(|_| conn.write_all(frame))
            .and_then(|_| conn.flush());
        match write_result {
            Ok(()) => {
                backoff.record_success(now);
                backoff.reset_after_idle(now);
            }
            Err(err) => {
                warn!("FemtoSocketHandler write failed: {err}");
                *connection = None;
                warn_drops(warner, |count| {
                    warn!("FemtoSocketHandler dropped {count} records due to write errors");
                });
                if let Some(delay) = backoff.next_sleep(now) {
                    thread::sleep(delay);
                }
            }
        }
    }
}

fn handle_flush_command(
    ack: Sender<()>,
    config: &SocketHandlerConfig,
    connection: &mut Option<ActiveConnection>,
) {
    let success = flush_connection(connection, config);
    let _ = ack.send(());
    if !success {
        warn!("FemtoSocketHandler flush requested without active connection");
    }
}

fn flush_connection(
    connection: &mut Option<ActiveConnection>,
    config: &SocketHandlerConfig,
) -> bool {
    match connection.as_mut() {
        Some(conn) => {
            let result = conn
                .set_write_timeout(config.write_timeout)
                .and_then(|_| conn.flush());
            if result.is_err() {
                *connection = None;
            }
            result.is_ok()
        }
        None => false,
    }
}

fn flush_silently(connection: &mut Option<ActiveConnection>, config: &SocketHandlerConfig) {
    if let Some(conn) = connection.as_mut() {
        let _ = conn.set_write_timeout(config.write_timeout);
        let _ = conn.flush();
    }
}

fn drain_pending(
    rx: &Receiver<SocketCommand>,
    config: &SocketHandlerConfig,
    connection: &mut Option<ActiveConnection>,
    backoff: &mut BackoffState,
    warner: &RateLimitedWarner,
) {
    loop {
        match rx.try_recv() {
            Ok(SocketCommand::Record(record)) => {
                handle_record_command(record, config, connection, backoff, warner)
            }
            Ok(SocketCommand::Flush(ack)) => {
                handle_flush_command(ack, config, connection);
            }
            Ok(SocketCommand::Shutdown(ack)) => {
                let _ = ack.send(());
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }
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
