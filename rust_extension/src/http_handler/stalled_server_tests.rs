//! A stalled-server test double for the shutdown-deadline drain contracts.
//!
//! The deadline contracts need a collector that accepts a connection and then
//! withholds its reply, and they need it to stay up for the whole test. This
//! module owns that server and the guard that stops it, so the drain tests can
//! state the contract rather than the thread and socket lifetime behind it.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

/// How long a stalled server's read loop waits before rechecking its stop flag.
///
/// Short enough that `Drop` joins in milliseconds, long enough that the polling
/// costs nothing while the test runs.
const STALL_POLL: Duration = Duration::from_millis(50);

/// A stalled server that stops when the test releases it.
///
/// Dropping the guard sets the stop flag, connects once to wake the blocked
/// `accept`, and joins the thread. Without it, a test leaves a thread and a
/// listener behind until the process exits, however the test ends.
pub(super) struct StalledServer {
    /// Address the listener is bound to.
    pub(super) addr: SocketAddr,
    /// Signals that a client sent a complete request.
    pub(super) received: mpsc::Receiver<()>,
    /// Set by the guard's `Drop` to ask both server loops to return.
    stop: Arc<AtomicBool>,
    /// Present until `Drop` joins the server thread.
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for StalledServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The accept loop may be blocked in `accept`, so a connection is what
        // wakes it. It carries no request, and the loop only has to read the
        // stop flag the connection just prompted.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Hold each connection open without answering it, one at a time.
///
/// The worker's request timeout then keeps each attempt in flight for the rest
/// of the close deadline, leaving queued records for the worker to count after
/// it aborts.
///
/// The listener stays open for the whole test rather than serving a single
/// connection. Serving one and then dropping the listener turns every later
/// attempt into an immediate connection refusal, and a worker that refuses
/// instantly drains the whole backlog inside its deadline instead of leaving
/// records for the abort to discard. Whether the listener has closed by the
/// time the next attempt starts is a thread-scheduling matter, so a
/// single-connection server made the deadline contract pass or fail on load.
///
/// # Examples
///
/// ```ignore
/// let server = spawn_stalled_server(listener)?;
/// server.received.recv_timeout(Duration::from_secs(2))?;
/// ```
pub(super) fn spawn_stalled_server(listener: TcpListener) -> io::Result<StalledServer> {
    let addr = listener.local_addr()?;
    let (request_tx, request_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            if thread_stop.load(Ordering::Relaxed) {
                return;
            }
            if !stall_connection(stream, &thread_stop, &request_tx) {
                return;
            }
        }
    });
    Ok(StalledServer {
        addr,
        received: request_rx,
        stop,
        thread: Some(thread),
    })
}

/// Read one request and hold the connection open until the client or the test ends.
///
/// Returns `false` when the stop flag was set and the accept loop should return,
/// `true` when the connection is done and the next one may be accepted.
fn stall_connection(
    mut stream: TcpStream,
    stop: &AtomicBool,
    request_tx: &mpsc::Sender<()>,
) -> bool {
    // A read timeout lets the loop notice the stop flag without closing the
    // connection: what this server withholds is its reply, not the socket, so
    // the client's own request timeout is what keeps each attempt in flight.
    let _ = stream.set_read_timeout(Some(STALL_POLL));
    if super::super::read_http_request(&mut stream).is_err() {
        return true;
    }
    let _ = request_tx.send(());
    hold_connection(&mut stream, stop)
}

/// Hold an open connection, abandoning it once the stop flag is set.
fn hold_connection(stream: &mut TcpStream, stop: &AtomicBool) -> bool {
    let mut sink = [0_u8; 1];
    loop {
        match classify_stall_read(stream.read(&mut sink)) {
            StallRead::More => {}
            StallRead::TimedOut if stop.load(Ordering::Relaxed) => return false,
            StallRead::TimedOut => {}
            StallRead::Closed => return true,
        }
    }
}

/// One step of a stalled server's read loop.
enum StallRead {
    /// The client sent more bytes.
    More,
    /// The poll interval elapsed with nothing to read.
    TimedOut,
    /// The client closed the connection, or the socket failed.
    Closed,
}

/// Classify a read on a connection whose timeout is the stall poll.
fn classify_stall_read(result: io::Result<usize>) -> StallRead {
    match result {
        Ok(0) => StallRead::Closed,
        Ok(_) => StallRead::More,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            StallRead::TimedOut
        }
        Err(_) => StallRead::Closed,
    }
}
