//! Worker thread driving HTTP I/O.
//!
//! The worker maintains a ureq Agent for connection pooling and handles
//! retries with exponential backoff for transient failures.

use std::{
    io,
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use log::warn;
use ureq::{Agent, AgentBuilder};

use super::{
    config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat},
    serialize::{serialize_json, serialize_url_encoded},
};
use crate::{
    handler::HandlerError,
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
    socket_handler::backoff::BackoffState,
};

/// Commands processed by the worker thread.
#[derive(Debug)]
pub enum HTTPCommand {
    Record(Box<FemtoLogRecord>),
    Flush(Sender<()>),
    Shutdown(Sender<()>),
}

/// Classification of HTTP response for retry logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseClass {
    /// 2xx responses - request succeeded.
    Success,
    /// 5xx, 429, or network errors - retry with backoff.
    Retryable,
    /// 4xx (except 429) - permanent failure, do not retry.
    Permanent,
}

/// Spawns a background worker thread to process HTTP commands.
///
/// The worker maintains a connection pool via `ureq::Agent` and handles
/// retries with exponential backoff for transient failures (5xx, 429).
///
/// # Arguments
///
/// * `config` - Configuration for the HTTP handler including URL, auth, and timeouts.
///
/// # Returns
///
/// A tuple containing:
/// * A sender for submitting [`HTTPCommand`]s to the worker
/// * A join handle for the spawned thread
pub fn spawn_worker(config: HTTPHandlerConfig) -> (Sender<HTTPCommand>, thread::JoinHandle<()>) {
    let (tx, rx) = bounded(config.capacity);
    let handle = thread::spawn(move || worker_loop(&rx, config));
    (tx, handle)
}

fn worker_loop(rx: &Receiver<HTTPCommand>, config: HTTPHandlerConfig) {
    Worker::new(config).run(rx);
}

struct Worker {
    config: HTTPHandlerConfig,
    agent: Agent,
    backoff: BackoffState,
    warner: RateLimitedWarner,
}

impl Worker {
    fn new(config: HTTPHandlerConfig) -> Self {
        let agent = AgentBuilder::new()
            .timeout_connect(config.connect_timeout)
            .timeout(config.write_timeout)
            .build();
        let backoff = BackoffState::new(config.backoff.clone());
        let warner = RateLimitedWarner::new(config.warn_interval);
        Self {
            config,
            agent,
            backoff,
            warner,
        }
    }

    fn handle_record_command(&mut self, record: &FemtoLogRecord) {
        let payload = match self.serialize_record(record) {
            Ok(p) => p,
            Err(err) => {
                warn!("FemtoHTTPHandler serialization error: {err}");
                self.warn_serialization_drops();
                return;
            }
        };
        self.send_request(&payload);
    }

    fn serialize_record(&self, record: &FemtoLogRecord) -> io::Result<String> {
        let fields = self.config.record_fields.as_deref();
        match self.config.format {
            SerializationFormat::UrlEncoded => serialize_url_encoded(record, fields),
            SerializationFormat::Json => serialize_json(record, fields),
        }
    }

    fn send_request(&mut self, payload: &str) {
        loop {
            let now = Instant::now();
            let result = self.execute_request(payload);
            if !self.continue_after_request(result, now) {
                return;
            }
        }
    }

    /// Record the request outcome and decide whether the retry loop continues.
    fn continue_after_request(
        &mut self,
        result: Result<ResponseClass, String>,
        now: Instant,
    ) -> bool {
        match result {
            Ok(ResponseClass::Success) => {
                self.backoff.record_success(now);
                false
            }
            Ok(ResponseClass::Retryable) => {
                self.sleep_and_should_retry("server returned retryable status", now)
            }
            Ok(ResponseClass::Permanent) => {
                warn!("FemtoHTTPHandler received permanent error (4xx), dropping record");
                self.warn_permanent_drops();
                false
            }
            Err(err) => self.sleep_and_should_retry(&err, now),
        }
    }

    fn execute_request(&self, payload: &str) -> Result<ResponseClass, String> {
        // Note: GET+JSON combination is rejected at build time by HTTPHandlerBuilder.
        let request = match self.config.method {
            HTTPMethod::GET => self.build_get_request(payload),
            HTTPMethod::POST => self.build_post_request(payload),
        };

        match request {
            Ok(response) => Ok(classify_status(response.status())),
            Err(err) => match *err {
                ureq::Error::Status(code, _) => Ok(classify_status(code)),
                ureq::Error::Transport(transport_err) => Err(transport_err.to_string()),
            },
        }
    }

    fn build_get_request(&self, payload: &str) -> Result<ureq::Response, Box<ureq::Error>> {
        let url = if self.config.url.contains('?') {
            format!("{}&{}", self.config.url, payload)
        } else {
            format!("{}?{}", self.config.url, payload)
        };
        let mut req = self.agent.get(&url);
        req = self.apply_auth(req);
        req = self.apply_headers(req);
        req.call().map_err(Box::new)
    }

    fn build_post_request(&self, payload: &str) -> Result<ureq::Response, Box<ureq::Error>> {
        let mut req = self.agent.post(&self.config.url);
        req = self.apply_auth(req);
        req = self.apply_headers(req);

        let content_type = match self.config.format {
            SerializationFormat::UrlEncoded => "application/x-www-form-urlencoded",
            SerializationFormat::Json => "application/json",
        };
        req = req.set("Content-Type", content_type);
        req.send_string(payload).map_err(Box::new)
    }

    fn apply_auth(&self, req: ureq::Request) -> ureq::Request {
        match &self.config.auth {
            AuthConfig::None => req,
            AuthConfig::Basic { username, password } => {
                let credentials = format!("{username}:{password}");
                let encoded = base64_encode(credentials.as_bytes());
                req.set("Authorization", &format!("Basic {encoded}"))
            }
            AuthConfig::Bearer { token } => req.set("Authorization", &format!("Bearer {token}")),
        }
    }

    fn apply_headers(&self, mut req: ureq::Request) -> ureq::Request {
        for (key, value) in &self.config.headers {
            req = req.set(key, value);
        }
        req
    }

    /// Handles a retryable error by logging, sleeping with backoff, and returning
    /// whether a retry should be attempted.
    ///
    /// Returns `true` if the caller should retry, `false` if the backoff deadline
    /// has been exceeded and the record should be dropped.
    fn sleep_and_should_retry(&mut self, err: &str, now: Instant) -> bool {
        warn!("FemtoHTTPHandler request failed: {err}");
        let Some(delay) = self.backoff.next_sleep(now) else {
            warn_drops(&self.warner, |count| {
                warn!("FemtoHTTPHandler dropped {count} records after exhausting retry deadline");
            });
            return false;
        };
        thread::sleep(delay);
        true
    }

    fn warn_serialization_drops(&self) {
        warn_drops(&self.warner, |count| {
            warn!("FemtoHTTPHandler dropped {count} records due to serialization failures");
        });
    }

    fn warn_permanent_drops(&self) {
        warn_drops(&self.warner, |count| {
            warn!("FemtoHTTPHandler dropped {count} records due to permanent errors");
        });
    }

    /// Best-effort acknowledgement for a completed worker operation.
    fn acknowledge(ack: &Sender<()>) {
        let Ok(()) = ack.send(()) else {
            // A timed-out caller may drop its receiver after the operation completes.
            return;
        };
    }

    /// Handles a flush command by immediately acknowledging completion.
    ///
    /// Unlike file or socket handlers, HTTP has no persistent connection or
    /// internal buffer to flush. Each request is sent synchronously in
    /// `send_request`, so there is no buffered data awaiting transmission.
    ///
    /// **Important**: This does not wait for in-flight retry attempts. If a
    /// record is currently in a backoff/retry loop, `flush()` returns
    /// immediately without blocking until those retries complete. Callers
    /// should not rely on `flush()` to guarantee delivery of records that
    /// encountered transient failures.
    fn handle_flush_command(ack: &Sender<()>) { Self::acknowledge(ack); }

    fn drain_pending(&mut self, rx: &Receiver<HTTPCommand>) {
        loop {
            match rx.try_recv() {
                Ok(HTTPCommand::Record(record)) => self.handle_record_command(&record),
                Ok(HTTPCommand::Flush(ack)) => Self::handle_flush_command(&ack),
                Ok(HTTPCommand::Shutdown(ack)) => Self::acknowledge(&ack),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    fn run(mut self, rx: &Receiver<HTTPCommand>) {
        loop {
            match rx.recv() {
                Ok(HTTPCommand::Record(record)) => self.handle_record_command(&record),
                Ok(HTTPCommand::Flush(ack)) => Self::handle_flush_command(&ack),
                Ok(HTTPCommand::Shutdown(ack)) => {
                    self.drain_pending(rx);
                    Self::handle_flush_command(&ack);
                    break;
                }
                Err(_) => {
                    self.drain_pending(rx);
                    break;
                }
            }
        }
    }
}

/// Classifies an HTTP status code for retry logic.
///
/// # Classification rules
///
/// * **2xx** → [`ResponseClass::Success`] - request completed successfully
/// * **429** → [`ResponseClass::Retryable`] - rate limited, retry with backoff
/// * **5xx** → [`ResponseClass::Retryable`] - server error, retry with backoff
/// * **Other** → [`ResponseClass::Permanent`] - client error (4xx except 429), do not retry
pub(crate) const fn classify_status(status: u16) -> ResponseClass {
    match status {
        200..=299 => ResponseClass::Success,
        429 | 500..=599 => ResponseClass::Retryable,
        _ => ResponseClass::Permanent,
    }
}

fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
}

/// Base64-encode a byte slice for Basic auth.
fn base64_encode(input: &[u8]) -> String { BASE64_STANDARD.encode(input) }

/// Enqueues a log record for transmission by the HTTP worker.
///
/// This is a non-blocking operation. If the queue is full, the record is
/// dropped and a rate-limited warning is emitted.
///
/// # Arguments
///
/// * `tx` - Sender channel to the HTTP worker thread
/// * `record` - The log record to enqueue
/// * `warner` - Rate-limited warner for drop notifications
///
/// # Errors
///
/// * [`HandlerError::QueueFull`] - The queue is at capacity; record was dropped
/// * [`HandlerError::Closed`] - The worker has shut down; record was dropped
pub fn enqueue_record(
    tx: &Sender<HTTPCommand>,
    record: FemtoLogRecord,
    warner: &RateLimitedWarner,
) -> Result<(), HandlerError> {
    match tx.try_send(HTTPCommand::Record(Box::new(record))) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            warner.record_drop();
            warner.warn_if_due(|count| {
                warn!("FemtoHTTPHandler queue full; dropped {count} records");
            });
            Err(HandlerError::QueueFull)
        }
        Err(TrySendError::Disconnected(_)) => {
            warner.record_drop();
            warner.warn_if_due(|count| {
                warn!("FemtoHTTPHandler disconnected; dropped {count} records");
            });
            Err(HandlerError::Closed)
        }
    }
}

/// Sends a flush command to the HTTP worker and waits for acknowledgement.
///
/// Uses a deadline-based approach to ensure the total wait time does not
/// exceed `timeout`, even if the send operation consumes part of the budget.
///
/// # Arguments
///
/// * `tx` - Sender channel to the HTTP worker thread
/// * `timeout` - Maximum time to wait for both sending and receiving the ack
///
/// # Returns
///
/// `true` if the flush was acknowledged within the timeout, `false` otherwise.
pub fn flush_queue(tx: &Sender<HTTPCommand>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(HTTPCommand::Flush(ack_tx), timeout)
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
