"""Check that an installed wheel is production-built and delivers records.

The location checks prove that the interpreter has loaded this environment's
native extension rather than the development build. They say nothing about
whether that extension works, so the probe also sends one record through the
native HTTP worker to a loopback endpoint. A wheel whose dependencies or TLS
or transport configuration are wrong passes the location checks and fails the
delivery check.
"""

from __future__ import annotations

import http.server
import importlib
import importlib.machinery
import queue
import sys
import threading
import typing as typ
from pathlib import Path

# The record whose delivery proves the installed native HTTP worker runs.
_PROBE_MESSAGE = "release-wheel-probe"
# The path the loopback collector expects the handler to post to.
_PROBE_PATH = "/probe"
# How long the delivered record may take to reach the loopback collector.
_DELIVERY_TIMEOUT_SECONDS = 10.0


def _native_location_errors(native_location: object) -> list[str]:
    """Return violations when a native module path is outside this environment.

    Parameters
    ----------
    native_location : object
        The value reported by the native module's ``__file__`` attribute.

    Returns
    -------
    list[str]
        Human-readable path or extension-suffix violations.

    Examples
    --------
    >>> _native_location_errors(None)
    ['native extension has no filesystem origin']
    """
    if not isinstance(native_location, str):
        return ["native extension has no filesystem origin"]

    native_path = Path(native_location).resolve()
    environment = Path(sys.prefix).resolve()
    errors: list[str] = []
    try:
        native_path.relative_to(environment)
    except ValueError:
        errors.append(f"native extension is outside sys.prefix: {native_path}")
    has_extension_suffix = any(
        native_path.name.endswith(suffix)
        for suffix in importlib.machinery.EXTENSION_SUFFIXES
    )
    if not has_extension_suffix:
        errors.append(f"native module does not use an extension suffix: {native_path}")
    return errors


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
    handler: typ.Any = None
    try:
        handler = femtologging.HTTPHandlerBuilder().with_endpoint(endpoint).build()
        handler.handle("release.wheel.probe", "INFO", _PROBE_MESSAGE)
        handler.flush()
        errors = _collect_delivery(collector, endpoint)
    finally:
        if handler is not None:
            handler.close()
        collector.shutdown()
        thread.join(timeout=1)
    return errors


def release_wheel_errors() -> list[str]:
    """Return checks that fail when this process is not using a release wheel.

    Returns
    -------
    list[str]
        Human-readable violations. An empty list means that the imported
        extension belongs to this environment, contains no test hooks, and
        delivered a record over HTTP.
    """
    import femtologging

    native = importlib.import_module("femtologging._femtologging_rs")
    errors = _native_location_errors(getattr(native, "__file__", None))

    hooks = (
        "set_timed_rotation_test_times_for_test",
        "clear_timed_rotation_test_times_for_test",
    )
    exposed = [name for name in hooks if hasattr(native, name)]
    if exposed:
        errors.append(f"native extension exposes test hooks: {', '.join(exposed)}")
    if femtologging._has_test_util is not False:
        errors.append("femtologging._has_test_util must be False")
    if not errors:
        errors.extend(http_behaviour_errors())
    return errors


def main() -> int:
    """Print release-wheel violations and return a process status.

    Returns
    -------
    int
        ``0`` when no violations are found; ``1`` when violations are found.

    Examples
    --------
    Run the probe after installing a release wheel in the active environment:

    >>> main()  # doctest: +SKIP
    release wheel probe passed
    0
    """
    errors = release_wheel_errors()
    if errors:
        for error in errors:
            print(f"release wheel probe: {error}", file=sys.stderr)
        return 1
    print("release wheel probe passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
