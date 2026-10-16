//! Worker thread driving HTTP I/O.
//!
//! The worker maintains a ureq Agent for connection pooling and handles
//! retries with exponential backoff for transient failures.
//!
//! Shutdown is carried on a [`ShutdownSignal`] rather than on the command
//! channel, because the command channel is where the backlog lives. A worker
//! that could only hear a shutdown by reading its queue would have to finish
//! the record in hand first, and against a collector that answers 429 forever
//! "the record in hand" is the rest of the process's life.
//!
//! The queue in front of this worker, and the classification of what comes
//! back, live in `queue.rs` and `response.rs`. What is left here is the part
//! that owns the clock: how long an attempt may run, and how long the loop
//! around it may go on.

use std::{io, sync::Arc, time::Instant};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use log::warn;
use ureq::{Agent, AgentBuilder};

use crate::{
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
    shutdown_signal::{ShutdownSignal, WaitOutcome},
    socket_handler::backoff::BackoffState,
};

use super::{
    config::{AuthConfig, HTTPHandlerConfig, HTTPMethod, SerializationFormat},
    queue::{HTTPCommand, discard_rest, warn_discarded, warn_drops},
    response::{ResponseClass, classify_status},
    serialize::{serialize_json, serialize_url_encoded},
};

/// Spawns a background worker thread to process HTTP commands.
///
/// The worker maintains a connection pool via `ureq::Agent` and handles
/// retries with exponential backoff for transient failures (5xx, 429).
///
/// # Arguments
///
/// * `config` - Configuration for the HTTP handler including URL, auth, timeouts, etc.
/// * `signal` - Shutdown state the handler moves on and the worker watches.
///   Shared rather than moved, so a close that gives up on the worker can
///   still tell it to stop.
///
/// # Returns
///
/// A tuple containing:
/// * A sender for submitting [`HTTPCommand`]s to the worker
/// * A join handle for the spawned thread
pub fn spawn_worker(
    config: HTTPHandlerConfig,
    signal: Arc<ShutdownSignal>,
) -> (Sender<HTTPCommand>, std::thread::JoinHandle<()>) {
    let (tx, rx) = bounded(config.capacity);
    let handle = std::thread::spawn(move || worker_loop(rx, config, signal));
    (tx, handle)
}

fn worker_loop(rx: Receiver<HTTPCommand>, config: HTTPHandlerConfig, signal: Arc<ShutdownSignal>) {
    Worker::new(config, signal).run(rx);
}

struct Worker {
    config: HTTPHandlerConfig,
    agent: Agent,
    backoff: BackoffState,
    warner: RateLimitedWarner,
    signal: Arc<ShutdownSignal>,
}

impl Worker {
    fn new(config: HTTPHandlerConfig, signal: Arc<ShutdownSignal>) -> Self {
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
            signal,
        }
    }

    fn handle_record_command(&mut self, record: FemtoLogRecord) {
        let payload = match self.serialize_record(&record) {
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
            if self.signal.is_aborted() {
                return;
            }
            let Some(agent) = self.agent_for_attempt() else {
                return;
            };
            let now = Instant::now();
            let result = self.execute_request(payload, &agent);
            match result {
                Ok(ResponseClass::Success) => {
                    self.backoff.record_success(now);
                    return;
                }
                Ok(ResponseClass::Retryable) => {
                    match self.should_retry("server returned retryable status", now) {
                        true => continue,
                        false => return,
                    }
                }
                Ok(ResponseClass::Permanent) => {
                    warn!("FemtoHTTPHandler received permanent error (4xx), dropping record");
                    self.warn_permanent_drops();
                    return;
                }
                Err(err) => match self.should_retry(&err, now) {
                    true => continue,
                    false => return,
                },
            }
        }
    }

    /// The agent to send the next attempt with, or `None` if there is no time
    /// left to send one.
    ///
    /// While the signal is running this is the pooled agent, so ordinary
    /// traffic keeps its connection reuse. Once a drain begins, a fresh agent
    /// is built around the remaining budget instead, because ureq's
    /// `timeout_connect` takes precedence over the per-request deadline when
    /// both are set: a pooled agent carrying the configured connect timeout
    /// would let one attempt run that timeout past the deadline the handler is
    /// waiting on, however short the request timeout it was handed.
    fn agent_for_attempt(&self) -> Option<Agent> {
        if !self.signal.is_shutting_down() {
            return Some(self.agent.clone());
        }
        let budget = self.signal.remaining()?;
        Some(
            AgentBuilder::new()
                .timeout_connect(budget.min(self.config.connect_timeout))
                .timeout(budget)
                .build(),
        )
    }

    fn execute_request(&self, payload: &str, agent: &Agent) -> Result<ResponseClass, String> {
        // Note: GET+JSON combination is rejected at build time by HTTPHandlerBuilder.
        let request = match self.config.method {
            HTTPMethod::GET => self.build_get_request(payload, agent),
            HTTPMethod::POST => self.build_post_request(payload, agent),
        };

        match request {
            Ok(response) => Ok(classify_status(response.status())),
            Err(err) => match *err {
                ureq::Error::Status(code, _) => Ok(classify_status(code)),
                ureq::Error::Transport(transport_err) => Err(transport_err.to_string()),
            },
        }
    }

    fn build_get_request(
        &self,
        payload: &str,
        agent: &Agent,
    ) -> Result<ureq::Response, Box<ureq::Error>> {
        let url = if self.config.url.contains('?') {
            format!("{}&{}", self.config.url, payload)
        } else {
            format!("{}?{}", self.config.url, payload)
        };
        let mut req = agent.get(&url);
        req = self.apply_auth(req);
        req = self.apply_headers(req);
        req.call().map_err(Box::new)
    }

    fn build_post_request(
        &self,
        payload: &str,
        agent: &Agent,
    ) -> Result<ureq::Response, Box<ureq::Error>> {
        let mut req = agent.post(&self.config.url);
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

    /// Logs a retryable failure, then says whether this record gets another
    /// attempt.
    ///
    /// A record is only retried while the signal is running. Any shutdown at
    /// all ends the retries for the record in hand, because the alternative is
    /// to hold the handler's acknowledgement until a backoff deadline the
    /// caller never asked for: a record that cannot be delivered inside the
    /// flush budget is a record the queue is about to discard anyway.
    ///
    /// The backoff sleep is the signal's, not `thread::sleep`'s, so a close
    /// that lands mid-delay is observed now rather than after it.
    fn should_retry(&mut self, err: &str, now: Instant) -> bool {
        warn!("FemtoHTTPHandler request failed: {err}");
        if self.signal.is_shutting_down() {
            self.warn_shutdown_drops();
            return false;
        }
        let Some(delay) = self.backoff.next_sleep(now) else {
            warn_drops(&self.warner, |count| {
                warn!("FemtoHTTPHandler dropped {count} records after exhausting retry deadline");
            });
            return false;
        };
        match self.signal.wait(delay) {
            WaitOutcome::Elapsed => true,
            WaitOutcome::ShutdownRequested | WaitOutcome::Aborted => {
                self.warn_shutdown_drops();
                false
            }
        }
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

    fn warn_shutdown_drops(&self) {
        warn_drops(&self.warner, |count| {
            warn!("FemtoHTTPHandler dropped {count} records during shutdown");
        });
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

    /// Attempt each queued command once, then acknowledge the shutdown.
    ///
    /// One attempt per record, with no retries: `should_retry` declines once
    /// the signal is shutting down, so a record here costs at most the single
    /// request the budget allows. Nothing is attempted at all once the
    /// deadline has passed, and the rest of the queue is discarded rather than
    /// sent, because a queue that outlasts its deadline is a queue nobody is
    /// still waiting for.
    fn drain_pending(&mut self, rx: &Receiver<HTTPCommand>, shutdown_ack: Option<Sender<()>>) {
        let mut discarded = 0_u64;
        loop {
            if self.signal.is_aborted() {
                discarded += discard_rest(rx);
                break;
            }
            match rx.try_recv() {
                Ok(HTTPCommand::Record(record)) => self.handle_record_command(record),
                Ok(HTTPCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(HTTPCommand::Shutdown(ack)) => {
                    let _ = ack.send(());
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        warn_discarded(discarded);
        // Last, so a handler waiting on the acknowledgement knows the queue
        // behind it has been dealt with.
        if let Some(ack) = shutdown_ack {
            let _ = ack.send(());
        }
    }

    fn run(mut self, rx: Receiver<HTTPCommand>) {
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
                Ok(HTTPCommand::Record(record)) => self.handle_record_command(record),
                Ok(HTTPCommand::Flush(ack)) => self.handle_flush_command(ack),
                Ok(HTTPCommand::Shutdown(ack)) => {
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

/// Base64-encode a byte slice for Basic auth.
fn base64_encode(input: &[u8]) -> String {
    BASE64_STANDARD.encode(input)
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
