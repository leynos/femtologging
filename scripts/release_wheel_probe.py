"""Check that an installed wheel is production-built and delivers records.

The location checks prove that the interpreter has loaded this environment's
native extension rather than the development build. They say nothing about
whether that extension works, so the probe also sends one record through the
native HTTP worker to a loopback endpoint. A wheel whose dependencies or TLS
or transport configuration are wrong passes the location checks and fails the
delivery check.
"""

from __future__ import annotations

import ast
import http.server
import importlib
import importlib.machinery
import queue
import sys
import threading
import typing as typ
from pathlib import Path

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from femtologging import FemtoHTTPHandler

# Test-only names that follow none of the shapes below.
_TEST_ONLY_EXACT_NAMES = frozenset({"_stream_waiter_count", "AttachmentIdentity"})
_TEST_ONLY_NAME_PREFIXES = ("_emit_rust_", "_install_test_")
_TEST_ONLY_NAME_SUFFIX = "_for_test"


def _is_test_only_name(name: str) -> bool:
    """Return whether *name* belongs to a test-only hook or helper.

    Parameters
    ----------
    name : str
        Attribute or stub declaration name to classify.

    Returns
    -------
    bool
        ``True`` for names that only a ``test-util`` build may expose.

    Examples
    --------
    >>> _is_test_only_name("wait_for_flush_entry_for_test")
    True
    >>> _is_test_only_name("setup_rust_logging")
    False
    """
    return (
        name in _TEST_ONLY_EXACT_NAMES
        or name.endswith(_TEST_ONLY_NAME_SUFFIX)
        or name.startswith(_TEST_ONLY_NAME_PREFIXES)
    )


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


def _native_surface_errors(native: object) -> list[str]:
    """Return violations for test-only attributes on the native surface.

    Both the module namespace and the ``FemtoLogger`` type are checked, because
    an attachment hook is a method rather than a module function.

    Parameters
    ----------
    native : object
        The imported native extension module.

    Returns
    -------
    list[str]
        Human-readable violations for module functions and logger methods.

    Examples
    --------
    >>> _native_surface_errors(object())
    []
    """
    errors: list[str] = []
    module_hooks = sorted(name for name in dir(native) if _is_test_only_name(name))
    if module_hooks:
        errors.append("native extension exposes test hooks: " + ", ".join(module_hooks))
    logger_type = getattr(native, "FemtoLogger", None)
    if logger_type is not None:
        method_hooks = sorted(
            name for name in dir(logger_type) if _is_test_only_name(name)
        )
        if method_hooks:
            errors.append(
                "FemtoLogger exposes test-only methods: " + ", ".join(method_hooks)
            )
    return errors


def _reexported_names(aliases: cabc.Iterable[ast.alias]) -> set[str]:
    """Return the names *aliases* re-export under an identical alias.

    Parameters
    ----------
    aliases : collections.abc.Iterable[ast.alias]
        Imported names from a ``from`` statement.

    Returns
    -------
    set[str]
        Names imported as themselves, which is how a stub re-exports them.

    Examples
    --------
    >>> _reexported_names([ast.alias(name="Token", asname="Token")])
    {'Token'}
    >>> _reexported_names([ast.alias(name="Token", asname=None)])
    set()
    """
    return {
        alias.asname
        for alias in aliases
        if alias.asname is not None and alias.asname == alias.name
    }


def _stub_declaration_names(tree: ast.AST) -> set[str]:
    """Return every name *tree* declares or explicitly re-exports.

    Parsing the stub rather than scanning lines keeps the check correct for
    syntax that is not a bare ``name:`` line, such as a PEP 695 generic class
    whose name is followed by ``[``. Type parameters are not declarations of
    the module's API, so they are not collected. A ``from x import Y as Y``
    alias is a stub's way of re-exporting ``Y``; a plain import stays private
    and is not collected.

    Parameters
    ----------
    tree : ast.AST
        Parsed type stub.

    Returns
    -------
    set[str]
        Names declared anywhere in the stub, including nested ones.

    Examples
    --------
    >>> sorted(_stub_declaration_names(ast.parse("class Token[T]: ...")))
    ['Token']
    >>> _stub_declaration_names(ast.parse("from .mod import Token as Token"))
    {'Token'}
    """
    names: set[str] = set()
    for node in ast.walk(tree):
        match node:
            case ast.AsyncFunctionDef() | ast.ClassDef() | ast.FunctionDef():
                names.add(node.name)
            case ast.AnnAssign(target=ast.Name(id=name)):
                names.add(name)
            case ast.Assign(targets=targets):
                names.update(
                    target.id for target in targets if isinstance(target, ast.Name)
                )
            case ast.TypeAlias(name=ast.Name(id=name)):
                names.add(name)
            case ast.ImportFrom(names=aliases):
                names.update(_reexported_names(aliases))
    return names


def _stub_surface_errors(package: object) -> list[str]:
    """Return violations for test-only declarations in the shipped stub.

    A stub is the published contract of the wheel. Declaring a hook that only a
    ``test-util`` build provides would promise production users an API they
    cannot call, so the stub must omit the same names as the native module.

    Parameters
    ----------
    package : object
        The imported ``femtologging`` package.

    Returns
    -------
    list[str]
        Human-readable violations found in ``_femtologging_rs.pyi``.

    Examples
    --------
    >>> _stub_surface_errors(object())
    ['femtologging package has no filesystem origin']
    """
    package_file = getattr(package, "__file__", None)
    if not isinstance(package_file, str):
        return ["femtologging package has no filesystem origin"]
    stub = Path(package_file).with_name("_femtologging_rs.pyi")
    if not stub.is_file():
        return [f"release wheel omits the type stub: {stub}"]
    try:
        declarations = _stub_declaration_names(
            ast.parse(stub.read_text(encoding="utf-8"))
        )
    except SyntaxError:
        return [f"release wheel type stub cannot be parsed: {stub}"]
    hooks = sorted(name for name in declarations if _is_test_only_name(name))
    if hooks:
        return ["type stub declares test-only hooks: " + ", ".join(hooks)]
    return []


def _resolve_probe_inputs(
    native: object | None, package: object | None
) -> tuple[object | None, object | None]:
    """Fill in the native module and package that the caller left unspecified.

    Parameters
    ----------
    native : object | None
        Injected native module, or ``None`` to import the installed one.
    package : object | None
        Injected ``femtologging`` package, or ``None`` to import it.

    Returns
    -------
    tuple[object | None, object | None]
        The native module and package to inspect.

    Examples
    --------
    >>> native = object()
    >>> _resolve_probe_inputs(native, object())[0] is native
    True
    """
    if package is None:
        import femtologging

        package = femtologging
    if native is None:
        native = importlib.import_module("femtologging._femtologging_rs")
    return native, package


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


def release_wheel_errors(
    *, native: object | None = None, package: object | None = None
) -> list[str]:
    """Return checks that fail when this process is not using a release wheel.

    Parameters
    ----------
    native : object | None
        Native extension module to inspect. Defaults to the module installed
        in this process; tests inject a stand-in so no shared import state is
        replaced.
    package : object | None
        ``femtologging`` package to inspect. Defaults to the imported package.

    Returns
    -------
    list[str]
        Human-readable violations. An empty list means that the imported
        extension belongs to this environment, contains no test hooks, delivered
        a record over HTTP, and that its stub declares only the production API.
    """
    native, package = _resolve_probe_inputs(native, package)
    errors = _native_location_errors(getattr(native, "__file__", None))
    errors.extend(_native_surface_errors(native))
    errors.extend(_stub_surface_errors(package))
    if getattr(package, "_has_test_util", None) is not False:
        errors.append("femtologging._has_test_util must be False")
    if not errors:
        errors.extend(http_behaviour_errors())
    return errors


def main(*, native: object | None = None, package: object | None = None) -> int:
    """Print release-wheel violations and return a process status.

    Parameters
    ----------
    native : object | None
        Native extension module to inspect. Defaults to the installed module.
    package : object | None
        ``femtologging`` package to inspect. Defaults to the imported package.

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
    errors = release_wheel_errors(native=native, package=package)
    if errors:
        for error in errors:
            print(f"release wheel probe: {error}", file=sys.stderr)
        return 1
    print("release wheel probe passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
