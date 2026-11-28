# Migration guide: v0.1.0 to v0.2.0

## Who should read this

Read this guide if an application uses femtologging 0.1 and is moving to 0.2.0.
Readers new to femtologging need none of it: start with the
[users' guide](users-guide.md) instead.

Most of 0.2.0 is new, opt-in capability, but some changes can need an edit
before an existing application behaves as it did. The ones most likely to
affect an application are the limits on logger names, the workers that loggers
now share, and what `disable_existing_loggers` does to the loggers a
configuration omits. Table 1 sorts every change by what it asks of the
application. A row that does not match the application can be skipped, and
every Python example here is run by the project's test suite, so it works as
printed.

Femtologging 0.2.0 is currently published as a beta, and installers skip
pre-releases unless asked. Request it explicitly, and pin the exact beta so
that moving to a later pre-release, which may bring further changes, is a
deliberate choice:

<!-- tested-example: migration-install -->
```bash
python -m pip install "femtologging==0.2.0b1"
```

The beta has one known issue: close each socket or HTTP handler from a single
thread, because a second thread's `flush()` or `close()` on the same handler
raises `RuntimeError` while the first `close()` is waiting. The
[users' guide](users-guide.md#operational-caveats) has the details.

## Impact at a glance

| Priority           | Area                                                                              | What to do                                                                         |
| ------------------ | --------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Required if called | [Logger names](#1-keep-logger-names-stable-and-bounded)                           | Stop deriving logger names from requests, users, or other data                     |
| Required if called | [Structured field names](#2-rename-structured-fields-that-use-a-reserved-name)    | Rename `extra` and `log_context()` fields that use a record field's name           |
| Required if called | [`BasicConfig`](#3-use-only-the-fields-of-basicconfig)                            | Keep application state out of `BasicConfig` instances                              |
| Required if called | [`RuntimeConfigBuilder`](#4-attach-everything-a-runtime-change-declares)          | Attach every handler and filter a runtime change declares                          |
| Required if called | [Test hooks](#5-stop-using-the-test-only-hooks)                                   | Build with `test-util` if any test-only hook is used                               |
| Required if called | [Rust API](#6-handle-the-fallible-rust-apis)                                      | Handle the new `Result` values, `flush_until()`, and `SchemaVersionError` variants |
| Review if enabled  | [`disable_existing_loggers`](#7-name-the-loggers-that-must-keep-emitting)         | Name every logger that must keep emitting                                          |
| Review if enabled  | [Thread-local state](#8-move-handler-state-off-threadinglocal)                    | Move handler state from `threading.local` to `contextvars`                         |
| Review if enabled  | [Standard library handler levels](#9-check-wrapped-handler-levels)                | Check the `level` of handlers wrapped in `StdlibHandlerAdapter`                    |
| Review if enabled  | [Failed configuration](#10-apply-a-corrected-configuration-after-a-failure)       | Stop relying on a failed configuration applying part of its changes                |
| Review if enabled  | [Network shutdown](#11-flush-network-handlers-before-closing-them)                | Flush socket and HTTP handlers before `close()` discards what is left              |
| Review if enabled  | [HTTP handlers](#12-fix-http-endpoints-and-headers-that-no-longer-build)          | Fix endpoints and headers that no longer build, and check redirects                |
| Review if enabled  | [Exception payloads](#13-check-payloads-passed-to-filter_frames)                  | Check very large or hand-built payloads passed to `filter_frames()`                |
| Review if enabled  | [Rust `log` macros](#14-guard-expensive-rust-log-arguments)                       | Guard expensive `log` macro arguments                                              |
| Review if enabled  | [Custom handler flushes](#15-return-a-truthful-result-from-handler-flush)         | Return `None` or `True` from a custom handler's `flush()` when it succeeds         |
| Review if enabled  | [File handler threads](#16-add-a-lock-around-file-handler-calls)                  | Add a lock where a file handler call kept other threads out                        |
| Review if enabled  | [Stream handler shutdown](#17-expect-stream-handlers-to-stop-waiting-sooner)      | Stop treating a slow stream `flush()` or `close()` as having finished              |
| Review if enabled  | [Multiline messages](#18-expect-line-breaks-in-messages-to-be-escaped)            | Update checks and parsers that expect raw line breaks in default output            |
| Review if enabled  | [Warning text](#19-match-the-new-warning-text)                                    | Update alerts that match femtologging's own warnings                               |
| Review if enabled  | [Untrusted configuration](#20-refuse-filter-factories-in-untrusted-configuration) | Pass `allow_factories=False` to `dictConfig()` for untrusted configuration         |
| Recommended        | [Handler filters](#21-move-application-wide-enrichment-to-the-handler)            | Attach application-wide enrichment to the shared handler                           |
| Optional           | [New features](#22-adopt-the-new-features)                                        | Adopt structured fields, formatters, rotation, and runtime changes as needed       |

*Table 1: Obligations when moving from v0.1.0 to v0.2.0.*

______________________________________________________________________

## 1. Keep logger names stable and bounded

Logger names now have limits. A name is limited to 255 UTF-8 bytes and 32
dot-separated segments, and may not contain control characters or the Unicode
line and paragraph separators. The process-wide registry holds at most 10,000
named loggers and never evicts one, so a pattern that creates one logger per
request eventually fails. The reasoning is in
[ADR-0007](adrs/adr-0007-bounded-logger-cardinality-and-shared-dispatch.md).

Use stable component names, and move identifiers to structured fields.

### Before: one logger per request

<!-- tested-example: migration-logger-names-before -->
```python
from femtologging import get_logger

request_id = 42
logger = get_logger(f"requests.{request_id}")
assert logger.info("handled") == "requests.42 [INFO] handled"
```

### After: a fixed name and a field

<!-- tested-example: migration-logger-names-after -->
```python
from femtologging import get_logger

request_id = 42
logger = get_logger("requests")
assert logger.info("handled", extra={"request_id": request_id}) is not None
```

Names used by the Rust `log` and `tracing` bridges fall back to the `root`
logger when they are refused. See [Name loggers](users-guide.md#name-loggers).

## 2. Rename structured fields that use a reserved name

The record's own field names are now reserved for structured fields. Using one,
such as `message`, `name`, `levelname`, or `logger`, as a key in `extra` or
`log_context()` raises `ValueError`, even when the record is below the logger's
level. In 0.1 such a field could shadow the real value in a handler or an HTTP
payload. Matching is exact and case-sensitive, so `message_id` and `Name` are
still fine. The full list is in
[Attach fields to a block of code](users-guide.md#attach-fields-to-a-block-of-code).

<!-- tested-example: migration-reserved-field -->
```python
from femtologging import get_logger

logger = get_logger("orders")

try:
    logger.info("order accepted", extra={"message": "accepted"})  # 0.1 allowed this
except ValueError:
    pass
else:
    raise AssertionError("a reserved name should be rejected")

assert logger.info("order accepted", extra={"message_id": "accepted"}) is not None
```

Rename the field, and rename it in any
`HTTPHandlerBuilder.with_record_fields()` list too, because that list matches
structured fields by name. A reserved name in that list now always selects the
built-in field.

Rust code sees the same rule: `push_log_context` returns
`LogContextError::ReservedKey`, and `log_with_metadata` drops a record whose
key-values use a reserved name and counts it in `get_context_dropped()`.

## 3. Use only the fields of `BasicConfig`

`BasicConfig` is now a slotted dataclass. An instance holds only `level`,
`filename`, `stream`, `force`, and `handlers`, so a mistyped or invented
attribute fails where it is assigned, instead of silently creating an attribute
that `basicConfig()` never reads. Keep any other state in a separate structure.

<!-- tested-example: migration-basic-config-slots -->
```python
from femtologging import BasicConfig

cfg = BasicConfig(level="INFO")
cfg.filename = "app.log"  # a declared field: unchanged

try:
    cfg.filename_backup = "app.log.bak"  # 0.1 silently accepted this
except AttributeError:
    pass
else:
    raise AssertionError("undeclared attributes should be rejected")

assert cfg.filename == "app.log"
```

## 4. Attach everything a runtime change declares

`RuntimeConfigBuilder.apply()` now rejects a handler or filter that no logger
will reference once the change is applied, before it builds anything, so a
stray declaration can no longer hold a file or worker thread open for the life
of the process. Attach each definition in the same transaction, or leave it out.

<!-- tested-example: migration-runtime-unreferenced -->
```python
from femtologging import (
    LoggerMutationBuilder,
    RuntimeConfigBuilder,
    StreamHandlerBuilder,
)

console = StreamHandlerBuilder.stderr()

try:
    RuntimeConfigBuilder().with_handler("console", console).apply()
except ValueError as error:
    assert "unreferenced definitions" in str(error)
else:
    raise AssertionError("an unattached handler should be rejected")

(
    RuntimeConfigBuilder()
    .with_handler("console", console)
    .with_logger("app", LoggerMutationBuilder().replace_handlers(["console"]))
    .apply()
)
```

## 5. Stop using the test-only hooks

Wheels and the source distribution are now built without the `test-util`
feature, and `femtologging._has_test_util` is `False`. The hooks that only the
project's own tests use are gone from them:

- `set_timed_rotation_test_times_for_test` and
  `clear_timed_rotation_test_times_for_test`, their underscore-prefixed
  wrappers, the rotating-handler `*_fresh_failure_for_test` helpers, and the
  other `*_for_test` wrappers in `femtologging._rust_compat`.
- `FemtoLogger.handler_ptrs_for_test()` and `filter_ptrs_for_test()`, which
  exposed memory addresses. Test builds offer `handler_identities_for_test()`
  and `filter_identities_for_test()`, which return opaque tokens that can be
  compared.
- The Rust emit and install hooks, such as `_emit_rust_log` and
  `_install_test_global_rust_logger`, which are also gone from the type stub.
- In Rust, `TestConfig` and `FemtoFileHandler::with_writer_for_test()`.

None of these were ever supported API. If code relies on them, set
`UV_CONFIG_FILE=.github/uv-test-util.toml` for a source install, or pass
`--features python,test-util` to a development build, adding `log-compat` or
`tracing-compat` for the Rust hooks. A test module that must also run against a
release wheel can look the hooks up with `getattr()`.

## 6. Handle the fallible Rust APIs

This affects only Rust code that uses femtologging directly. Handlers attached
from Python are unaffected.

- `FemtoLogger::new()` returns `PyResult<Self>` and rejects an invalid name.
- `FemtoLogger::with_parent()` returns `Result<Self, DispatcherError>` and
  reports a dispatch pool that could not start. `DispatcherError` is
  re-exported from the crate root.
- `FemtoLogger::add_handler()` returns `Result<(), HandlerError>`.
- A handler whose `is_python_backed()` returns `true` must also return `true`
  from `provides_context_dispatch()` and override `handle_with_context()` to
  invoke Python inside the supplied context. Otherwise `add_handler()` fails
  with `HandlerError::MissingContextDispatch`.
- `FemtoLogger::flush_handlers()` no longer calls a handler's synchronous
  `flush()`. A custom handler takes part only if it overrides
  `FemtoHandlerTrait::flush_until(context)`, waiting no later than the deadline
  it is given and returning `false` when it cannot confirm the flush. Without
  that override, `flush_handlers()` returns `false`.
- `SchemaVersionError` is now `#[non_exhaustive]` and has the new variants
  `Nested`, `TreeTooDeep`, and `TreeTooLarge`, so add a wildcard arm to every
  `match` on it. `validate_version()` now checks every nested `cause`,
  `context`, and exception-group member, so a payload that passed before can
  now fail. See
  [Validate exception schema versions](users-guide.md#validate-exception-schema-versions).

## 7. Name the loggers that must keep emitting

A configuration with `disable_existing_loggers` enabled now disables every
logger it omits, instead of only clearing its handlers. A disabled logger
rejects records before the level check, filters, and dispatch, so it no longer
reaches ancestor handlers through propagation. This setting defaults to on in
`fileConfig()`, so check any application that uses it.

### Before: an omitted logger still reached `root`

<!-- tested-example: migration-disable-existing-before -->
```python
from femtologging import dictConfig, get_logger

noisy = get_logger("noisy")
dictConfig({
    "version": 1,
    "disable_existing_loggers": True,
    "handlers": {"console": {"class": "logging.StreamHandler"}},
    "root": {"level": "ERROR", "handlers": ["console"]},
})

# In 0.1 this still reached the root handler. Now nothing happens.
assert noisy.error("dropped") is None
assert noisy.disabled
assert noisy.get_disabled_drops() == 1
```

### After: name the logger, or leave existing loggers alone

<!-- tested-example: migration-disable-existing-after -->
```python
from femtologging import dictConfig, get_logger

noisy = get_logger("noisy")
dictConfig({
    "version": 1,
    "disable_existing_loggers": True,
    "handlers": {"console": {"class": "logging.StreamHandler"}},
    "loggers": {"noisy": {"level": "ERROR"}},
    "root": {"level": "ERROR", "handlers": ["console"]},
})

assert noisy.error("still seen") is not None
assert get_logger("root").flush_handlers()
```

To keep every existing logger as it is, set `disable_existing_loggers` to
`False` instead. That does not re-enable a logger an earlier configuration
disabled: name it in a later configuration to enable it again. Its kept
ancestors keep whatever state they had. See
[Disable loggers a configuration omits](users-guide.md#disable-loggers-a-configuration-omits).

## 8. Move handler state off `threading.local`

Loggers no longer own worker threads. A process starts a fixed pool of one to
four dispatch workers, and loggers on the same shard share a worker, though
each logger's records stay in order. Three things follow:

- A custom handler's `threading.local` state may be seen by handlers serving
  other loggers. Use `contextvars` for values from the logging caller, or keep
  the state on the handler.
- A slow Python handler now delays every logger on its shard.
- Creating a logger can raise `RuntimeError` if the process has reached its
  thread limit. The next logger creation tries again.

Python handlers, including `StdlibHandlerAdapter`, now also run inside a copy
of the `contextvars` context that was active when the record was logged. A
handler that read worker-thread state through a `ContextVar` now sees the
caller's value:

<!-- tested-example: migration-handler-contextvars -->
```python
import contextvars

from femtologging import get_logger

tenant = contextvars.ContextVar("tenant", default="none")


class TenantCollector:
    def __init__(self) -> None:
        self.tenants: list[str] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        self.tenants.append(tenant.get())


logger = get_logger("billing")
collector = TenantCollector()
logger.add_handler(collector)

tenant.set("acme")
logger.info("invoice issued")
assert logger.flush_handlers()
assert collector.tenants == ["acme"]
```

## 9. Check wrapped handler levels

`StdlibHandlerAdapter` now honours the wrapped handler's `level`, as the
standard library does. In 0.1 a handler with `setLevel(logging.ERROR)` still
received every record the logger accepted, so it may now receive fewer. Lower
the level, or leave it at `logging.NOTSET`, if the handler should see
everything. The adapter reads the level as it processes each record, so
`setLevel()` needs no rebuild. See
[Reuse standard library handlers](users-guide.md#reuse-standard-library-handlers).

## 10. Apply a corrected configuration after a failure

`ConfigBuilder.build_and_init()`, `dictConfig()`, and `fileConfig()` now
prepare everything before changing any live logger. A call that raises leaves
every logger exactly as it was. If start-up code caught a configuration error
and carried on, expecting the valid parts to have applied, apply a corrected
configuration instead.

<!-- tested-example: migration-config-transaction -->
```python
from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)

ConfigBuilder().with_root_logger(
    LoggerConfigBuilder().with_level("ERROR")
).build_and_init()

broken = (
    ConfigBuilder()
    .with_handler("console", StreamHandlerBuilder.stderr())
    .with_root_logger(
        LoggerConfigBuilder().with_level("DEBUG").with_handlers(["missing"])
    )
)
try:
    broken.build_and_init()
except Exception:
    pass
else:
    raise AssertionError("an unknown handler reference should be rejected")

assert get_logger("root").level == "ERROR"
```

Three side effects sit outside this guarantee: records already queued are not
retracted, a `dictConfig()` filter factory has already run, and building a
handler can open its file or socket before a later error rejects the
configuration.

## 11. Flush network handlers before closing them

`FemtoSocketHandler.close()` and `FemtoHTTPHandler.close()` now return within
the handler's write timeout, and so does dropping either handler. In 0.1, a
worker stuck on an unresponsive collector or endpoint could hold `close()` for
much longer. The deadline changes what reaches the other end:

- **Each queued record gets one attempt.** Shutdown does not retry or back off.
- **Records still queued at the deadline are discarded.** A warning reports
  how many, such as
  `FemtoHTTPHandler discarded 12 queued records at its shutdown deadline`. The
  worker no longer carries on sending them in the background.
- **A socket frame cut off by the deadline stays cut off.** The collector may
  receive a truncated frame, and the connection is not reused.
- **A socket `flush()` waits for one write timeout, not two.** Waiting for
  queue space and waiting for the worker's answer now share one budget, so a
  stalled collector makes `flush()` return `False` sooner.

If shutdown relied on `close()` waiting until every record was delivered, call
`flush()` first and check its result:

<!-- tested-example: migration-network-shutdown -->
```python
from femtologging import HTTPHandlerBuilder

handler = (
    HTTPHandlerBuilder()
    .with_endpoint("http://127.0.0.1:9/ingest")
    .with_write_timeout_ms(200)
    .build()
)
flushed = handler.flush()
handler.close()

# Nothing was queued, so the flush succeeded. Had records been waiting on a
# slow endpoint, False would warn that close() may discard some of them.
assert flushed is True
```

## 12. Fix HTTP endpoints and headers that no longer build

`HTTPHandlerBuilder` now checks at build time what the HTTP client will send.
In 0.1, a bad endpoint or header built a handler that then failed on every
record:

- An endpoint that is not an absolute `http` or `https` URL raises
  `HandlerConfigError`, and so does a header name, header value, or token that
  cannot be sent as an HTTP header. The error names the endpoint or header but
  never repeats a header's value.

<!-- tested-example: migration-http-endpoint -->
```python
from femtologging import HandlerConfigError, HTTPHandlerBuilder

try:
    HTTPHandlerBuilder().with_endpoint("ftp://logs.example.com/ingest").build()
except HandlerConfigError as error:
    assert "the scheme must be http or https" in str(error)
else:
    raise AssertionError("a non-HTTP endpoint should be rejected")
```

The transport changed too:

- At most four redirects are followed, all within one write timeout, and
  `Authorization` and `Cookie` are no longer sent to the redirect target. A
  300, and a 307 or 308 answering a POST, now fail without a retry. If the
  collector answers that way, point the handler at the final collector
  endpoint, or make the collector answer the POST itself with a 2xx. Do not
  switch to a 301, 302, or 303: the handler follows those with a GET that
  carries no record, so a 2xx at the target would count as delivered.
  `Authorization` and `Cookie` are dropped on a redirect, but other configured
  headers, such as `X-Api-Key`, are still sent to the target.
- The write timeout now limits connecting as well, so a connection waits for
  at most the smaller of the connect and write timeouts. Raise the write
  timeout if connecting legitimately takes longer.
- Certificates are checked against the WebPKI roots built into femtologging. A
  private certificate authority is not supported.

See [Send records over HTTP](users-guide.md#send-records-over-http).

## 13. Check payloads passed to `filter_frames()`

`filter_frames()` keeps its call pattern and adds keyword-only
`max_exception_depth` (default 1,000) and `max_exception_nodes` (default
10,000) limits. Existing calls still work, but a cyclic payload, a malformed
one, or a graph beyond the limits now raises an exception. If a legitimate
payload exceeds the defaults, pass larger limits. See
[Trim frames from exception and stack payloads](users-guide.md#trim-frames-from-exception-and-stack-payloads).

## 14. Guard expensive Rust `log` arguments

The Rust `log` bridge now decides `log::log_enabled!` per target, using that
target's current femtologging level, so a target at `DEBUG` accepts `DEBUG`
records even while another is held at `ERROR`. The `log` crate still evaluates
a macro's arguments for a target held below the process-wide maximum, so add an
explicit `log::log_enabled!` guard around expensive ones. See
[Bridge the log and tracing crates](users-guide.md#bridge-the-log-and-tracing-crates).

## 15. Return a truthful result from handler `flush()`

`flush_handlers()` now calls the `flush()` method of a handler written in
Python. In 0.1 it never did, so a handler that buffered records was not asked
to write them. The call runs on the dispatch worker, like `handle()`, so it
cannot rely on state belonging to the thread that called `flush_handlers()`.
Its result now counts:

- Returning `None`, as `logging.Handler.flush()` does, or a true value counts
  as success.
- Returning `False`, `0`, or another false value, or raising, makes
  `flush_handlers()` return `False`. The other handlers are still flushed. An
  exception also logs a short warning that leaves out its text.
- A handler without a callable `flush()` method counts as flushed.

One two-second deadline covers the whole `flush_handlers()` call. Handlers
still waiting when it expires are not flushed, and `flush_handlers()` can return
`False` while a slow `flush()` is still running, so keep the method as short
and defensive as `handle()`. `get_flush_diagnostics()` reports which stage
failed; see
[Handle back-pressure and dropped records](users-guide.md#handle-back-pressure-and-dropped-records).

<!-- tested-example: migration-handler-flush -->
```python
from femtologging import get_logger


class Buffered:
    def __init__(self) -> None:
        self.pending: list[str] = []
        self.written: list[str] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        self.pending.append(message)

    def flush(self) -> None:
        self.written.extend(self.pending)
        self.pending.clear()


logger = get_logger("orders")
handler = Buffered()
logger.add_handler(handler)
logger.info("order 7 shipped")

assert logger.flush_handlers()
assert handler.written == ["order 7 shipped"]
```

A flush also covers only the records queued when it starts. Stop the threads
that log before flushing and closing handlers at shutdown; see
[Shut down cleanly](users-guide.md#shut-down-cleanly).

## 16. Add a lock around file handler calls

Explicit `handle()`, `flush()`, and `close()` calls on `FemtoFileHandler`,
`FemtoRotatingFileHandler`, and `FemtoTimedRotatingFileHandler` now release the
GIL while they wait. Other Python threads keep running during slow file I/O,
which also means one of these calls no longer keeps them out. If code relied on
that, add a lock.

Two related changes:

- Once a `close()` has started, other threads see a closed handler: `handle()`
  raises `RuntimeError` (`ValueError` from the timed-rotating handler),
  `flush()` returns `False`, and a second `close()` waits for the first.
- `close()` now waits at most one second for the worker, then logs a warning
  and returns. Call it explicitly at shutdown: the destructor holds the GIL
  while it waits.

See
[Concurrency and the GIL for file handlers](users-guide.md#concurrency-and-the-gil-for-file-handlers).

## 17. Expect stream handlers to stop waiting sooner

`FemtoStreamHandler` applies its flush timeout, set with
`StreamHandlerBuilder.with_flush_after_ms()`, once per call. The file handlers'
`flush()` likewise shares its one second between handing over the request and
waiting for the answer:

- **`flush()` waits one timeout, not two.** Handing over the request and
  waiting for the answer used to get a full timeout each. A flush that
  succeeded only in that second window now returns `False`.
- **`close()` returns at the timeout.** It no longer joins the worker thread,
  so a slow thread-local destructor cannot hold it up. The worker can still be
  running, and writing to the stream, after `close()` returns.

`close()` returns `None` either way. If shutdown must know that the worker
finished, or that a thread-local destructor ran, signal that from application
code.

## 18. Expect line breaks in messages to be escaped

The default rendering, `"{logger} [{level}] {message}"`, now writes line breaks
in the message as escapes such as `\n` and `\u{2028}`, so each record's message
stays on one line and cannot forge another record. Update anything that
expected the raw line break: test assertions, parsers that count lines as
records, and `max_bytes` settings sized for the shorter output. Where the raw
text is needed, use a `FormatterBuilder` format or a callable formatter, which
receive the message unchanged:

<!-- tested-example: migration-multiline-message -->
```python
from femtologging import FileHandlerBuilder, FormatterBuilder, get_logger

raw_format = FormatterBuilder().with_format("%(name)s [%(levelname)s] %(message)s")
default = FileHandlerBuilder("default.log").build()
raw = FileHandlerBuilder("raw.log").with_formatter(raw_format).build()

logger = get_logger("core")
logger.add_handler(default)
logger.add_handler(raw)
logger.info("first\nsecond")
assert logger.flush_handlers()
default.close()
raw.close()

with open("default.log", encoding="utf-8") as log_file:
    assert log_file.read() == "core [INFO] first\\nsecond\n"
with open("raw.log", encoding="utf-8") as log_file:
    assert log_file.read() == "core [INFO] first\nsecond\n"
```

Stack traces and exceptions still follow the message on their own lines and are
not escaped. See [Multiline messages](users-guide.md#multiline-messages).

## 19. Match the new warning text

If alerts match femtologging's own warnings, update the patterns:

| Warning                     | 0.1 wording                                                                                                                                 | 0.2.0 wording                                                                                                         |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Stream handler drops        | `FemtoStreamHandler: N log records dropped in the last interval`                                                                            | `FemtoStreamHandler: N log records dropped since the last report`                                                     |
| HTTP handler drops          | `FemtoHTTPHandler dropped N records in the last interval`, or a sentence per cause such as `FemtoHTTPHandler queue full; dropped N records` | `FemtoHTTPHandler dropped N records since the last report`, then `; triggering cause: ...` when one drop prompted it  |
| Socket handler drops        | The same forms, starting `FemtoSocketHandler`                                                                                               | `FemtoSocketHandler dropped N records since the last report`, with the same suffix                                    |
| Logger drops at destruction | None                                                                                                                                        | `FemtoLogger: dropped N records; reported by flush`, or `FemtoLogger: dropped N records after context capture failed` |
| HTTP request failures       | The HTTP client's own error text, with the endpoint                                                                                         | A fixed category, such as `transport error (DNS lookup failed)`, without the endpoint                                 |

*Table 2: Warnings whose wording changed in v0.2.0.*

A drop count now covers every drop since the previous report, including those
held back while warnings were rate-limited, and `flush()`, `close()`, or
destruction reports any count still held back. An HTTP failure no longer names
the endpoint, so identify the handler instead; each handler has exactly one.

## 20. Refuse filter factories in untrusted configuration

`dictConfig()` gains a keyword-only `allow_factories` option. A `"()"` filter
entry imports and calls whatever its configuration names, so it is as powerful
as the application's own code. The default, `True`, keeps the standard
library's behaviour. If any configuration comes from tenants, an API, a remote
service, or a writable file, pass `allow_factories=False` to reject `"()"`
entries before anything is imported. It is not a sandbox: handlers still open
the destinations the configuration names. See
[Filter factories run code](users-guide.md#filter-factories-run-code).

## 21. Move application-wide enrichment to the handler

Logger filters do not run for records propagated from child loggers, so a
filter on `root` misses records logged through `shop.api`. Handler builders now
accept `with_filters()`, and such filters run for every record routed to the
handler. The accepted decision is recorded in
[ADR-0003](adrs/adr-0003-python-stdlib-filter-parity.md#accepted-addendum-2026-10-14-handler-filters).

### Before: a filter on each logger

<!-- tested-example: migration-handler-filter-before -->
```python
import logging

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
)


def enrich(record: logging.LogRecord) -> bool:
    record.service = "shop"
    return True


(
    ConfigBuilder()
    .with_filter("enrich", PythonCallbackFilterBuilder(enrich))
    .with_logger("shop.api", LoggerConfigBuilder().with_filters(["enrich"]))
    .with_logger("shop.worker", LoggerConfigBuilder().with_filters(["enrich"]))
    .with_root_logger(LoggerConfigBuilder())
    .build_and_init()
)
assert get_logger("shop.api").info("accepted") is not None
```

### After: one filter on the shared handler

<!-- tested-example: migration-handler-filter-after -->
```python
import logging

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
)


def enrich(record: logging.LogRecord) -> bool:
    record.service = "shop"
    return True


(
    ConfigBuilder()
    .with_filter("enrich", PythonCallbackFilterBuilder(enrich))
    .with_handler(
        "file",
        FileHandlerBuilder("app.log")
        .with_filters(["enrich"])
        .with_formatter(FormatterBuilder().with_format("%(service)s %(message)s")),
    )
    .with_root_logger(LoggerConfigBuilder().with_handlers(["file"]))
    .build_and_init()
)

get_logger("shop.api").info("accepted")
assert get_logger("root").flush_handlers()
with open("app.log", encoding="utf-8") as log_file:
    assert log_file.read() == "shop accepted\n"
```

## 22. Adopt the new features

Everything in this section is additive, so existing logging calls keep working.

- **Structured fields.** `log()` and the level methods accept a keyword-only
  `extra` mapping of scalar values. See
  [Attach fields to one record](users-guide.md#attach-fields-to-one-record).
- **Scoped context.** `log_context(**fields)` attaches fields to every record
  logged inside a `with` block, and is safe across `await`. See
  [Attach fields to a block of code](users-guide.md#attach-fields-to-a-block-of-code).
  Rust scoped context stays thread-local, and the two never mix; see
  [Rust and Python context](users-guide.md#rust-and-python-context).
- **`%`-style formatters.** `FormatterBuilder` now works, directly or by name
  through `ConfigBuilder` and `dictConfig()`. See
  [Format output](users-guide.md#format-output).
- **Callback filters.** `PythonCallbackFilterBuilder` turns a callable, or an
  object with a `filter(record)` method, into a filter, and `dictConfig()`
  accepts the `"()"` factory syntax. See
  [Filter and enrich records](users-guide.md#filter-and-enrich-records).
- **Time-based rotation.** See
  [Rotate files on a schedule](users-guide.md#rotate-files-on-a-schedule).
- **Runtime reconfiguration.** See
  [Change configuration at runtime](users-guide.md#change-configuration-at-runtime).
- **Counters.** `get_context_dropped()` counts records discarded because Rust
  context failed validation, `get_context_capture_failures()` counts failed
  `contextvars` snapshots, and `get_disabled_drops()` counts records refused by
  a disabled logger. All are separate from `get_dropped()`.
- **Reliable flushing.** `flush_handlers()` and each built-in handler's
  `flush()` and `close()` now release the global interpreter lock while they
  wait. A call made straight after logging could previously stall until its
  timeout and report failure, `close()` could lose records still queued, and a
  slow network shutdown froze every other Python thread.
- **Wait diagnostics.** File handlers report how their `handle()`, `flush()`,
  and `close()` waits ended through `wait_diagnostics()`. See
  [Concurrency and the GIL for file handlers](users-guide.md#concurrency-and-the-gil-for-file-handlers).
- **Flush diagnostics.** `get_flush_diagnostics()` counts each logger's
  `flush_handlers()` attempts, failures by stage, and durations. See
  [Handle back-pressure and dropped records](users-guide.md#handle-back-pressure-and-dropped-records).
- **Declarative-only `dictConfig()`.** `allow_factories=False` rejects `"()"`
  filter factories. See
  [Filter factories run code](users-guide.md#filter-factories-run-code).

<!-- tested-example: migration-extra -->
```python
from femtologging import get_logger, log_context

logger = get_logger("service")
with log_context(request_id=42):
    assert logger.info("accepted", extra={"user": "alice"}) == "service [INFO] accepted"
```

## What does not need to change

Valid logger names and the hierarchy, levels, the `basicConfig()` signature,
the built-in handlers' constructors, and the Python handler protocol
(`handle()` and `handle_record()`) are unchanged. Configuration that worked in
0.1 keeps working, apart from the rows in Table 1. Code written before 0.1.0
should first follow the [0.1.0 migration guide](v0-1-0-migration-guide.md).

## Migration checklist

- [ ] Search for logger names built from data, such as f-strings passed to
  `get_logger()`.
- [ ] Rename `extra` and `log_context()` fields that use a reserved name, and
  update `with_record_fields()` lists to match.
- [ ] Remove undeclared attributes from `BasicConfig` instances.
- [ ] Attach every handler and filter declared on a `RuntimeConfigBuilder`.
- [ ] Remove any reliance on test-only hooks in production builds.
- [ ] Update Rust code for the fallible `FemtoLogger` constructors and
  `add_handler()`, implement `flush_until()` on custom handlers, and add a
  wildcard arm to matches on `SchemaVersionError`.
- [ ] Check each use of `disable_existing_loggers`, including the `fileConfig()`
  default.
- [ ] Move handler state from `threading.local` to `contextvars`.
- [ ] Check the `level` of each handler wrapped in `StdlibHandlerAdapter`.
- [ ] Remove any reliance on partially applied configuration.
- [ ] Flush socket and HTTP handlers before closing them, and check what
  `flush()` returns.
- [ ] Build each HTTP handler once against 0.2.0 to catch rejected endpoints
  and headers, and check how the collector redirects.
- [ ] Add `log::log_enabled!` guards around expensive Rust `log` arguments.
- [ ] Check that custom handlers' `flush()` methods return `None` or `True`
  when they succeed, and do not rely on the calling thread.
- [ ] Add a lock wherever a file handler call was relied on to keep other
  threads out.
- [ ] Check that shutdown does not assume a stream handler's worker has
  finished when `close()` returns.
- [ ] Update tests, parsers, and `max_bytes` settings that expect raw line
  breaks in default output.
- [ ] Update alerts that match femtologging's warning text.
- [ ] Pass `allow_factories=False` wherever `dictConfig()` reads configuration
  that is not fully trusted.
- [ ] Consider moving application-wide enrichment to handler filters.

## Common errors and fixes

- **Error:** `logger name cannot exceed 255 bytes; got 256 bytes`, or
  `logger name cannot exceed 32 dot-separated segments; got 33`
  - **Fix:** Use a stable component name and put identifiers in `extra`. See
    [task 1](#1-keep-logger-names-stable-and-bounded).
- **Error:**
  `logger registry limit reached: at most 10000 distinct loggers …`
  - **Fix:** Find the code creating loggers from request or user data. See
    [task 1](#1-keep-logger-names-stable-and-bounded).
- **Error:** `could not start the log dispatch pool: …` (`RuntimeError`)
  - **Fix:** The process has reached its thread limit. Reduce its thread use or
    raise the limit; the next logger creation tries again.
- **Error:**
  `'BasicConfig' object has no attribute 'filename_backup' …`
  - **Fix:** Keep the value in a separate structure. See
    [task 3](#3-use-only-the-fields-of-basicconfig).
- **Error:**
  `invalid runtime mutation: unreferenced definitions: handlers ["console"] …`
  - **Fix:** See [task 4](#4-attach-everything-a-runtime-change-declares).
- **Error:**
  `context key 'message' is reserved and cannot be used for structured fields`
  (`ValueError`)
  - **Fix:** Rename the field. See
    [task 2](#2-rename-structured-fields-that-use-a-reserved-name).
- **Error:** `AttributeError` or `ImportError` naming a `*_for_test` hook
  - **Fix:** Build with `test-util`. See
    [task 5](#5-stop-using-the-test-only-hooks).
- **Error:** `Python-backed handlers must provide contextual dispatch`
  - **Fix:** Implement contextual dispatch on the Rust handler. See
    [task 6](#6-handle-the-fallible-rust-apis).
- **Error:** `non-exhaustive patterns` in a Rust `match` on
  `SchemaVersionError`
  - **Fix:** Add a wildcard arm. See [task 6](#6-handle-the-fallible-rust-apis).
- **Error:** `invalid HTTP endpoint: the scheme must be http or https`, or
  `invalid value for HTTP header x-api-key` (`HandlerConfigError`)
  - **Fix:** Correct the endpoint or header. See
    [task 12](#12-fix-http-endpoints-and-headers-that-no-longer-build).
- **Error:**
  `filter 'svc' uses a '()' factory, rejected by allow_factories=False`
  (`ValueError`)
  - **Fix:** Replace the factory with a `level` or `name` filter, or register
    the filter in code. See
    [task 20](#20-refuse-filter-factories-in-untrusted-configuration).
- **Error:** `basicConfig() got an unexpected keyword argument 'format'`
  - **Fix:** Build a handler with a `FormatterBuilder` and pass it through
    `handlers`. See [Format output](users-guide.md#format-output).
- **Error:** `… invalid handler configuration: unknown formatter id: plain`
  - **Fix:** Register the formatter with `ConfigBuilder.with_formatter()` or in
    `dictConfig()`'s `formatters` section.
- **Error:** `exception payload graph contains a cycle at $.cause`, or
  `exception payload graph exceeds max_exception_depth=1000`
  - **Fix:** Correct the payload, or pass larger limits. See
    [task 13](#13-check-payloads-passed-to-filter_frames).
- **Warning:**
  `FemtoSocketHandler discarded 12 queued records at its shutdown deadline`, or
  the same from `FemtoHTTPHandler`
  - **Fix:** Call `flush()` before `close()`, or raise the write timeout. See
    [task 11](#11-flush-network-handlers-before-closing-them).
