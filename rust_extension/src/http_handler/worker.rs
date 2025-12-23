//! Worker thread driving HTTP I/O.
//!
//! The worker maintains a ureq Agent for connection pooling and handles
//! retries with exponential backoff for transient failures.

use std::{
    io, thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use log::warn;
use ureq::{Agent, AgentBuilder};

use crate::{
    handler::HandlerError, log_record::FemtoLogRecord, rate_limited_warner::RateLimitedWarner,
    socket_handler::backoff::BackoffState,
};

use super::{
    config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat},
    serialise::{serialise_json, serialise_url_encoded},
};

/// Commands processed by the worker thread.
#[derive(Debug)]
pub enum HTTPCommand {
    Record(FemtoLogRecord),
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
/// * `config` - Configuration for the HTTP handler including URL, auth, timeouts, etc.
///
/// # Returns
///
/// A tuple containing:
/// * A sender for submitting [`HTTPCommand`]s to the worker
/// * A join handle for the spawned thread
pub fn spawn_worker(config: HTTPHandlerConfig) -> (Sender<HTTPCommand>, thread::JoinHandle<()>) {
    let (tx, rx) = bounded(config.capacity);
    let handle = thread::spawn(move || worker_loop(rx, config));
    (tx, handle)
}

fn worker_loop(rx: Receiver<HTTPCommand>, config: HTTPHandlerConfig) {
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

    fn handle_record_command(&mut self, record: FemtoLogRecord) {
        let payload = match self.serialise_record(&record) {
            Ok(p) => p,
            Err(err) => {
                warn!("FemtoHTTPHandler serialization error: {err}");
                warn_drops(&self.warner, |count| {
                    warn!("FemtoHTTPHandler dropped {count} records due to serialization failures");
                });
                return;
            }
        };
        self.send_request(&payload);
    }

    fn serialise_record(&self, record: &FemtoLogRecord) -> io::Result<String> {
        let fields = self.config.record_fields.as_deref();
        match self.config.format {
            SerializationFormat::UrlEncoded => serialise_url_encoded(record, fields),
            SerializationFormat::Json => serialise_json(record, fields),
        }
    }

    fn send_request(&mut self, payload: &str) {
        loop {
            let now = Instant::now();
            let result = self.execute_request(payload);
            match result {
                Ok(ResponseClass::Success) => {
                    self.backoff.record_success(now);
                    return;
                }
                Ok(ResponseClass::Retryable) => {
                    if !self.sleep_and_should_retry("server returned retryable status", now) {
                        return;
                    }
                }
                Ok(ResponseClass::Permanent) => {
                    warn!("FemtoHTTPHandler received permanent error (4xx), dropping record");
                    warn_drops(&self.warner, |count| {
                        warn!("FemtoHTTPHandler dropped {count} records due to permanent errors");
                    });
                    return;
                }
                Err(err) => {
                    if !self.sleep_and_should_retry(&err, now) {
                        return;
                    }
                }
            }
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
                let credentials = format!("{}:{}", username, password);
                let encoded = base64_encode(credentials.as_bytes());
                req.set("Authorization", &format!("Basic {}", encoded))
            }
            AuthConfig::Bearer { token } => req.set("Authorization", &format!("Bearer {}", token)),
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
    fn handle_flush_command(&mut self, ack: Sender<()>) {
        // Ignore send error: if the receiver has dropped, there's nothing to do.
        let _ = ack.send(());
    }

    fn drain_pending(&mut self, rx: &Receiver<HTTPCommand>) {
        loop {
            match rx.try_recv() {
                Ok(HTTPCommand::Record(record)) => self.handle_record_command(record),
                Ok(HTTPCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(HTTPCommand::Shutdown(ack)) => {
                    let _ = ack.send(());
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn run(mut self, rx: Receiver<HTTPCommand>) {
        loop {
            match rx.recv() {
                Ok(HTTPCommand::Record(record)) => self.handle_record_command(record),
                Ok(HTTPCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(HTTPCommand::Shutdown(ack)) => {
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
pub(crate) fn classify_status(status: u16) -> ResponseClass {
    match status {
        200..=299 => ResponseClass::Success,
        429 => ResponseClass::Retryable,
        500..=599 => ResponseClass::Retryable,
        _ => ResponseClass::Permanent,
    }
}

fn warn_drops(warner: &RateLimitedWarner, log: impl FnMut(u64)) {
    warner.record_drop();
    warner.warn_if_due(log);
}

/// Base64-encode a byte slice for Basic auth.
fn base64_encode(input: &[u8]) -> String {
    BASE64_STANDARD.encode(input)
}

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
    match tx.try_send(HTTPCommand::Record(record)) {
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

/// Sends a flush command to the HTTP worker and waits for acknowledgment.
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
mod tests {
    use super::*;

    #[test]
    fn classify_2xx_as_success() {
        assert_eq!(classify_status(200), ResponseClass::Success);
        assert_eq!(classify_status(201), ResponseClass::Success);
        assert_eq!(classify_status(204), ResponseClass::Success);
    }

    #[test]
    fn classify_5xx_as_retryable() {
        assert_eq!(classify_status(500), ResponseClass::Retryable);
        assert_eq!(classify_status(502), ResponseClass::Retryable);
        assert_eq!(classify_status(503), ResponseClass::Retryable);
    }

    #[test]
    fn classify_429_as_retryable() {
        assert_eq!(classify_status(429), ResponseClass::Retryable);
    }

    #[test]
    fn classify_4xx_as_permanent() {
        assert_eq!(classify_status(400), ResponseClass::Permanent);
        assert_eq!(classify_status(401), ResponseClass::Permanent);
        assert_eq!(classify_status(403), ResponseClass::Permanent);
        assert_eq!(classify_status(404), ResponseClass::Permanent);
    }

    #[test]
    fn base64_encode_basic() {
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }
}
