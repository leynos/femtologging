//! Worker thread driving socket I/O.

use std::{thread, time::Instant};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
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
    while let Ok(cmd) = rx.recv() {
        match cmd {
            SocketCommand::Record(record) => {
                match serialise_record(&record).and_then(|payload| {
                    frame_payload(&payload, config.max_frame_size).ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large")
                    })
                }) {
                    Ok(frame) => {
                        let now = Instant::now();
                        if connection.is_none() {
                            match connect_transport(&config.transport, config.connect_timeout) {
                                Ok(mut stream) => {
                                    let _ = stream.set_write_timeout(config.write_timeout);
                                    backoff.record_success(now);
                                    connection = Some(stream);
                                }
                                Err(err) => {
                                    warner.record_drop();
                                    warner.warn_if_due(|count| {
                                        warn!(
                                            "FemtoSocketHandler failed to connect: {err}; dropped {count} records"
                                        );
                                    });
                                    if let Some(delay) = backoff.next_sleep(now) {
                                        thread::sleep(delay);
                                    }
                                    continue;
                                }
                            }
                        }

                        let mut send_frame = |conn: &mut ActiveConnection| -> std::io::Result<()> {
                            conn.set_write_timeout(config.write_timeout)?;
                            conn.write_all(&frame)?;
                            conn.flush()
                        };

                        match connection.as_mut().map(&mut send_frame) {
                            Some(Ok(())) => {
                                backoff.record_success(now);
                                backoff.reset_after_idle(now);
                            }
                            Some(Err(err)) => {
                                warn!("FemtoSocketHandler write failed: {err}");
                                connection = None;
                                warner.record_drop();
                                warner.warn_if_due(|count| {
                                    warn!(
                                        "FemtoSocketHandler dropped {count} records due to write errors"
                                    );
                                });
                                if let Some(delay) = backoff.next_sleep(now) {
                                    thread::sleep(delay);
                                }
                            }
                            None => {
                                warner.record_drop();
                                warner.warn_if_due(|count| {
                                    warn!(
                                        "FemtoSocketHandler dropped {count} records; no active connection"
                                    );
                                });
                            }
                        }
                    }
                    Err(err) => {
                        warn!("FemtoSocketHandler serialisation error: {err}");
                        warner.record_drop();
                        warner.warn_if_due(|count| {
                            warn!(
                                "FemtoSocketHandler dropped {count} records due to serialisation failures"
                            );
                        });
                    }
                }
            }
            SocketCommand::Flush(ack) => {
                let success = connection
                    .as_mut()
                    .map(|conn| conn.flush().is_ok())
                    .unwrap_or(false);
                let _ = ack.send(());
                if !success {
                    warn!("FemtoSocketHandler flush requested without active connection");
                }
            }
        }
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

pub fn flush_queue(tx: &Sender<SocketCommand>, timeout: std::time::Duration) -> bool {
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(SocketCommand::Flush(ack_tx), timeout)
        .is_err()
    {
        return false;
    }
    ack_rx.recv_timeout(timeout).is_ok()
}
