//! Test harness for the HTTP transport feedback regression.
//!
//! The regression needs three things that must not share a process with the
//! logging configuration under test: a local HTTP collector, a watchdog
//! around the child that owns that configuration, and an observable request
//! stream. A separate process is what makes the global Rust logger, the
//! manager registry and the worker thread uncontaminated by other tests.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// How long the collector waits for the child to make its next request.
const REQUEST_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// One HTTP request the local collector received.
#[derive(Clone, Debug)]
pub(crate) struct RequestObservation {
    /// Request method as sent on the request line.
    pub(crate) method: String,
    /// Request target, including any query string.
    pub(crate) path: String,
    /// Request body, empty for a GET.
    pub(crate) body: String,
}

impl RequestObservation {
    /// Return the path and body as one searchable payload.
    pub(crate) fn payload(&self) -> String {
        format!("{} {}", self.path, self.body)
    }

    /// Return whether the payload contains `needle`.
    pub(crate) fn contains(&self, needle: &str) -> bool {
        self.payload().contains(needle)
    }
}

/// Parse a request's header lines and its content length.
fn read_headers(reader: &mut BufReader<TcpStream>) -> io::Result<usize> {
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            return Ok(content_length);
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
}

/// Read one complete request from an accepted connection.
fn read_request(stream: &mut TcpStream) -> io::Result<RequestObservation> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let content_length = read_headers(&mut reader)?;
    let mut body = vec![0; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    Ok(RequestObservation {
        method,
        path,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// A local HTTP collector that records requests until asked to stop.
pub(crate) struct TestCollector {
    /// Address the child must send requests to.
    addr: SocketAddr,
    /// Set when the collector should leave its accept loop.
    stop: Arc<AtomicBool>,
    /// Set once the request count exceeds the configured hard cap.
    overflowed: Arc<AtomicBool>,
    /// Number of requests observed, including ones above the hard cap.
    count: Arc<AtomicUsize>,
    /// Observations delivered to the test that owns the collector.
    requests: mpsc::Receiver<RequestObservation>,
    /// Thread serving requests until `stop` is raised.
    handle: Option<thread::JoinHandle<()>>,
}

impl TestCollector {
    /// Bind an ephemeral local port and start serving empty `200` replies.
    pub(crate) fn spawn(hard_cap: usize) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;

        let stop = Arc::new(AtomicBool::new(false));
        let overflowed = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let (tx, requests) = mpsc::channel();

        let thread_stop = Arc::clone(&stop);
        let thread_overflowed = Arc::clone(&overflowed);
        let thread_count = Arc::clone(&count);
        let handle = thread::spawn(move || {
            serve_requests(
                listener,
                &thread_stop,
                &thread_overflowed,
                &thread_count,
                hard_cap,
                |request| {
                    let _ = tx.send(request);
                },
            );
        });

        Ok(Self {
            addr,
            stop,
            overflowed,
            count,
            requests,
            handle: Some(handle),
        })
    }

    /// Return the address the child should target.
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Return whether more requests arrived than the hard cap allows.
    pub(crate) fn has_overflowed(&self) -> bool {
        self.overflowed.load(Ordering::SeqCst)
    }

    /// Return the number of requests observed so far.
    pub(crate) fn request_count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Wait up to `timeout` for another request.
    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<RequestObservation, mpsc::RecvTimeoutError> {
        self.requests.recv_timeout(timeout)
    }

    /// Stop accepting requests and wait for the serving thread to finish.
    pub(crate) fn stop_and_join(mut self, timeout: Duration) -> Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        join_with_deadline(handle, timeout)
    }
}

impl Drop for TestCollector {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = join_with_deadline(handle, Duration::from_secs(2));
        }
    }
}

/// Accept and answer requests until the stop flag is raised.
fn serve_requests<F>(
    listener: TcpListener,
    stop: &AtomicBool,
    overflowed: &AtomicBool,
    count: &AtomicUsize,
    hard_cap: usize,
    mut observe: F,
) where
    F: FnMut(RequestObservation),
{
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let Ok(request) = read_request(&mut stream) else {
                    continue;
                };
                let seen = count.fetch_add(1, Ordering::SeqCst) + 1;
                if seen > hard_cap {
                    overflowed.store(true, Ordering::SeqCst);
                }
                observe(request);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                let _ = stream.flush();
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(REQUEST_POLL_INTERVAL);
            }
            Err(_) => break,
        }
    }
}

/// One finished child process and the output it produced.
pub(crate) struct ChildRun {
    /// Exit status, absent only when the watchdog had to kill the child.
    pub(crate) status: Option<ExitStatus>,
    /// Whether the watchdog stopped the child at its deadline.
    pub(crate) timed_out: bool,
    /// Everything the child wrote to standard output.
    pub(crate) stdout: String,
    /// Everything the child wrote to standard error.
    pub(crate) stderr: String,
}

impl ChildRun {
    /// Return whether the child exited successfully without the watchdog.
    pub(crate) fn succeeded(&self) -> bool {
        !self.timed_out && self.status.as_ref().is_some_and(ExitStatus::success)
    }

    /// Return a failure report that includes both captured streams.
    pub(crate) fn failure_report(&self) -> String {
        format!(
            "child did not complete cleanly (status={:?}, timed_out={})\nstdout:\n{}\nstderr:\n{}",
            self.status, self.timed_out, self.stdout, self.stderr
        )
    }
}

/// Run one ignored test in a fresh copy of this integration test binary.
pub(crate) fn run_ignored_test(test_name: &str, timeout: Duration) -> io::Result<ChildRun> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--ignored", "--exact", test_name, "--nocapture"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    if let Some((name, value)) = loader_search_path() {
        command.env(name, value);
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("child stdout is piped");
    let stderr = child.stderr.take().expect("child stderr is piped");
    let stdout_reader = thread::spawn(move || read_stream(stdout));
    let stderr_reader = thread::spawn(move || read_stream(stderr));

    let (status, timed_out) = wait_with_deadline(&mut child, timeout)?;
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(ChildRun {
        status,
        timed_out,
        stdout,
        stderr,
    })
}

/// Read a captured pipe to a string, treating invalid UTF-8 as recoverable.
fn read_stream(stream: impl Read) -> String {
    let mut bytes = Vec::new();
    let mut stream = stream;
    let _ = stream.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Reap the child, killing it if the deadline passes first.
fn wait_with_deadline(
    child: &mut Child,
    timeout: Duration,
) -> io::Result<(Option<ExitStatus>, bool)> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((Some(status), false));
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let status = child.wait()?;
            return Ok((Some(status), true));
        }
        thread::sleep(REQUEST_POLL_INTERVAL);
    }
}

/// Join a thread, giving up after `timeout`.
fn join_with_deadline(handle: thread::JoinHandle<()>, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return Err(format!("thread did not finish within {timeout:?}"));
        }
        thread::sleep(REQUEST_POLL_INTERVAL);
    }
    handle
        .join()
        .map_err(|_| "server thread panicked".to_string())
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: Cargo's LD_LIBRARY_PATH is required to load Python in the isolated test child"
)]
fn loader_search_path() -> Option<(&'static str, OsString)> {
    std::env::var_os("LD_LIBRARY_PATH").map(|value| ("LD_LIBRARY_PATH", value))
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: Cargo's dynamic-library search path is required to load Python in the isolated test child"
)]
fn loader_search_path() -> Option<(&'static str, OsString)> {
    std::env::var_os("DYLD_FALLBACK_LIBRARY_PATH")
        .map(|value| ("DYLD_FALLBACK_LIBRARY_PATH", value))
}

#[cfg(windows)]
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: Cargo's PATH is required to load Python in the isolated test child"
)]
fn loader_search_path() -> Option<(&'static str, OsString)> {
    std::env::var_os("PATH").map(|value| ("PATH", value))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn loader_search_path() -> Option<(&'static str, OsString)> {
    None
}
