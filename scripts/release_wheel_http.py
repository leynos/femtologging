"""HTTP delivery check for the release-wheel probe.

The probe sends one record through the installed native HTTP worker to a
loopback endpoint. A wheel whose dependencies or TLS or transport configuration
are wrong passes the location checks and fails this check. The module imports
nothing from the repository and does not import ``femtologging`` at module
scope, so the probe can run it under ``python -I`` from an empty directory.
"""

from __future__ import annotations

import http.server
import queue
import threading
import typing as typ

if typ.TYPE_CHECKING:
    from femtologging import FemtoHTTPHandler

# The record whose delivery proves the installed native HTTP worker runs.
_PROBE_MESSAGE = "release-wheel-probe"
# The path the loopback collector expects the handler to post to.
_PROBE_PATH = "/probe"
# How long the delivered record may take to reach the loopback collector.
_DELIVERY_TIMEOUT_SECONDS = 10.0


class _RecordCollector(http.server.ThreadingHTTPServer):
    """Loopback endpoint that records the record the handler delivers.

    The native handler posts on its own worker thread, so the probe needs a
    live endpoint to receive the request and a queue to wait on for it.
    """

    allow_reuse_address = True
    daemon_threads = True

    def __init__(self) -> None:
        """Bind an ephemeral loopback port and create the request queue."""
        super().__init__(("127.0.0.1", 0), _RecordRequestHandler)
        self.requests: queue.Queue[tuple[str, bytes]] = queue.Queue()


class _RecordRequestHandler(http.server.BaseHTTPRequestHandler):
    """Record each delivered POST body and answer with a success status."""

    def do_POST(self) -> None:
        """Store the delivered request, then reply 204 No Content."""
        collector = typ.cast("_RecordCollector", self.server)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        collector.requests.put((self.path, body))
        self.send_response(http.HTTPStatus.NO_CONTENT)
        self.end_headers()

    @typ.override
    def log_message(
        self,
        format: str,
        *arguments: object,
    ) -> None:
        """Silence the default per-request stderr logging."""
        _ = (format, arguments)


def _collector_endpoint(collector: _RecordCollector) -> str:
    """Return the endpoint URL a handler should post probe records to."""
    host, port = collector.server_address[:2]
    return f"http://{host}:{port}{_PROBE_PATH}"


def _collect_delivery(collector: _RecordCollector, endpoint: str) -> list[str]:
    """Wait for the probe record and report a missing or malformed delivery."""
    try:
        path, body = collector.requests.get(timeout=_DELIVERY_TIMEOUT_SECONDS)
    except queue.Empty:
        return [f"no HTTP delivery reached {endpoint}"]
    errors: list[str] = []
    if path != _PROBE_PATH:
        errors.append(f"the record used the wrong request path: {path}")
    if _PROBE_MESSAGE.encode() not in body:
        errors.append("the delivered body did not contain the record message")
    return errors


def http_behaviour_errors() -> list[str]:
    """Deliver one record through the installed wheel's native HTTP worker.

    Returns
    -------
    list[str]
        Human-readable violations. An empty list means the handler built from
        this installation reached a loopback endpoint and delivered its record,
        which the location checks alone cannot show.

    Examples
    --------
    Run after installing a wheel built without ``test-util``:

    >>> http_behaviour_errors()  # doctest: +SKIP
    []
    """
    import femtologging

    collector = _RecordCollector()
    thread = threading.Thread(target=collector.serve_forever, daemon=True)
    thread.start()
    endpoint = _collector_endpoint(collector)
    handler: FemtoHTTPHandler | None = None
    try:
        handler = femtologging.HTTPHandlerBuilder().with_endpoint(endpoint).build()
        handler.handle("release.wheel.probe", "INFO", _PROBE_MESSAGE)
        handler.flush()
        errors = _collect_delivery(collector, endpoint)
    finally:
        if handler is not None:
            handler.close()
        collector.shutdown()
        collector.server_close()
        thread.join(timeout=1)
    return errors
