//! Worker thread driving HTTP I/O.
//!
//! The worker maintains a ureq Agent for connection pooling and handles
//! retries with exponential backoff for transient failures.

use std::{
    io, thread,
    time::{Duration, Instant},
};

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
                warn!("FemtoHTTPHandler serialisation error: {err}");
                warn_drops(&self.warner, |count| {
                    warn!("FemtoHTTPHandler dropped {count} records due to serialisation failures");
                });
                return;
            }
        };
        let now = Instant::now();
        self.send_request(&payload, now);
    }

    fn serialise_record(&self, record: &FemtoLogRecord) -> io::Result<String> {
        let fields = self.config.record_fields.as_deref();
        match self.config.format {
            SerializationFormat::UrlEncoded => serialise_url_encoded(record, fields),
            SerializationFormat::Json => serialise_json(record, fields),
        }
    }

    fn send_request(&mut self, payload: &str, now: Instant) {
        let result = self.execute_request(payload);
        match result {
            Ok(class) => match class {
                ResponseClass::Success => {
                    self.backoff.record_success(now);
                    self.backoff.reset_after_idle(now);
                }
                ResponseClass::Retryable => {
                    self.handle_retryable_error("server returned retryable status", now);
                }
                ResponseClass::Permanent => {
                    warn!("FemtoHTTPHandler received permanent error (4xx), dropping record");
                    warn_drops(&self.warner, |count| {
                        warn!("FemtoHTTPHandler dropped {count} records due to permanent errors");
                    });
                }
            },
            Err(err) => {
                self.handle_retryable_error(&err, now);
            }
        }
    }

    fn execute_request(&self, payload: &str) -> Result<ResponseClass, String> {
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

    fn handle_retryable_error(&mut self, err: &str, now: Instant) {
        warn!("FemtoHTTPHandler request failed: {err}");
        warn_drops(&self.warner, |count| {
            warn!("FemtoHTTPHandler dropped {count} records due to request failures");
        });
        self.sleep_if_backing_off(now);
    }

    fn sleep_if_backing_off(&mut self, now: Instant) {
        if let Some(delay) = self.backoff.next_sleep(now) {
            thread::sleep(delay);
        }
    }

    fn handle_flush_command(&mut self, ack: Sender<()>) {
        // HTTP has no persistent connection to flush in the traditional sense.
        // We simply acknowledge the flush request.
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

fn classify_status(status: u16) -> ResponseClass {
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

/// Simple Base64 encoder for Basic auth.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        let combined = (b0 << 16) | (b1 << 8) | b2;

        result.push(ALPHABET[(combined >> 18) & 0x3F] as char);
        result.push(ALPHABET[(combined >> 12) & 0x3F] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[(combined >> 6) & 0x3F] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[combined & 0x3F] as char);
        } else {
            result.push('=');
        }
    }

    result
}

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

pub fn flush_queue(tx: &Sender<HTTPCommand>, timeout: Duration) -> bool {
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(HTTPCommand::Flush(ack_tx), timeout)
        .is_err()
    {
        return false;
    }
    ack_rx.recv_timeout(timeout).is_ok()
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
