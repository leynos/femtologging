# Migration guide: v0.1.0 to v0.2.0

## Who should read this

Read this guide if you have an application using femtologging 0.1 and are
moving to 0.2.0. If you are new to femtologging, you need none of it: start
with the [users' guide](users-guide.md) instead.

Most of 0.2.0 is new, opt-in capability, but some changes can need an edit
before an existing application behaves as it did. The ones most likely to
affect you are the limits on logger names, the workers that loggers now share,
and what `disable_existing_loggers` does to the loggers a configuration omits.
Table 1 sorts every change by what it asks of you. A row that does not match
your application can be skipped, and every Python example here is run by the
project's test suite, so it works as printed.

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

| Priority           | Area                                                                       | What to do                                                                       |
| ------------------ | -------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Required if called | [Logger names](#1-keep-logger-names-stable-and-bounded)                    | Stop deriving logger names from requests, users, or other data                   |
| Required if called | [`BasicConfig`](#2-use-only-the-fields-of-basicconfig)                     | Keep your own state out of `BasicConfig` instances                               |
| Required if called | [`RuntimeConfigBuilder`](#3-attach-everything-a-runtime-change-declares)   | Attach every handler and filter a runtime change declares                        |
| Required if called | [Test hooks](#4-stop-using-the-timed-rotation-test-hooks)                  | Build with `test-util` if you use the timed-rotation test hooks                  |
| Required if called | [Rust API](#5-handle-the-fallible-rust-apis)                               | Handle the new `Result` values in Rust code                                      |
| Review if enabled  | [`disable_existing_loggers`](#6-name-the-loggers-that-must-keep-emitting)  | Name every logger that must keep emitting                                        |
| Review if enabled  | [Thread-local state](#7-move-handler-state-off-threadinglocal)             | Move handler state from `threading.local` to `contextvars`                       |
| Review if enabled  | [Standard library handler levels](#8-check-wrapped-handler-levels)         | Check the `level` of handlers wrapped in `StdlibHandlerAdapter`                  |
| Review if enabled  | [Failed configuration](#9-apply-a-corrected-configuration-after-a-failure) | Stop relying on a failed configuration applying part of its changes              |
| Review if enabled  | [Network shutdown](#10-flush-network-handlers-before-closing-them)         | Flush socket and HTTP handlers before `close()` discards what is left            |
| Review if enabled  | [Exception payloads](#11-check-payloads-passed-to-filter_frames)           | Check very large or hand-built payloads passed to `filter_frames()`              |
| Review if enabled  | [Rust `log` macros](#12-guard-expensive-rust-log-arguments)                | Guard expensive `log` macro arguments                                            |
| Recommended        | [Handler filters](#13-move-application-wide-enrichment-to-the-handler)     | Attach application-wide enrichment to the shared handler                         |
| Optional           | [New features](#14-adopt-the-new-features)                                 | Adopt structured fields, formatters, rotation, and runtime changes when you like |

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
logger when they are refused. See
[Name your loggers](users-guide.md#name-your-loggers).

## 2. Use only the fields of `BasicConfig`

`BasicConfig` is now a slotted dataclass. An instance holds only `level`,
`filename`, `stream`, `force`, and `handlers`, so a mistyped or invented
attribute fails where you assign it, instead of silently creating an attribute
that `basicConfig()` never reads. Keep any other state in your own structure.

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

## 3. Attach everything a runtime change declares

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

## 4. Stop using the timed-rotation test hooks

Wheels and the source distribution are now built without the `test-util`
feature, so they no longer export `set_timed_rotation_test_times_for_test` or
`clear_timed_rotation_test_times_for_test`, and `femtologging._has_test_util` is
`False`. The underscore-prefixed Python wrappers remain as shims: the setter
raises `RuntimeError`, and the clearer does nothing.

These helpers were never supported API. If you rely on them, set
`UV_CONFIG_FILE=.github/uv-test-util.toml` for a source install, or pass
`--features python,test-util` to a development build.

## 5. Handle the fallible Rust APIs

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

## 6. Name the loggers that must keep emitting

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

To keep every existing logger, set `disable_existing_loggers` to `False`
instead. Naming a disabled logger in a later configuration enables it again;
its kept ancestors keep whatever state they had. See
[Disable loggers a configuration omits](users-guide.md#disable-loggers-a-configuration-omits).

## 7. Move handler state off `threading.local`

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

## 8. Check wrapped handler levels

`StdlibHandlerAdapter` now honours the wrapped handler's `level`, as the
standard library does. In 0.1 a handler with `setLevel(logging.ERROR)` still
received every record the logger accepted, so it may now receive fewer. Lower
the level, or leave it at `logging.NOTSET`, if the handler should see
everything. The adapter reads the level as it processes each record, so
`setLevel()` needs no rebuild. See
[Reuse standard library handlers](users-guide.md#reuse-standard-library-handlers).

## 9. Apply a corrected configuration after a failure

`ConfigBuilder.build_and_init()`, `dictConfig()`, and `fileConfig()` now
prepare everything before changing any live logger. A call that raises leaves
every logger exactly as it was. If your start-up code caught a configuration
error and carried on, expecting the valid parts to have applied, apply a
corrected configuration instead.

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

Two side effects sit outside this guarantee: records already queued are not
retracted, and building a handler can open its file or socket before a later
error rejects the configuration.

## 10. Flush network handlers before closing them

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

If your shutdown relied on `close()` waiting until every record was delivered,
call `flush()` first and check its result:

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

## 11. Check payloads passed to `filter_frames()`

`filter_frames()` keeps its call pattern and adds keyword-only
`max_exception_depth` (default 1,000) and `max_exception_nodes` (default
10,000) limits. Existing calls still work, but a cyclic payload, a malformed
one, or a graph beyond the limits now raises an exception. If a legitimate
payload exceeds the defaults, pass larger limits. See
[Trim frames from exception and stack payloads](users-guide.md#trim-frames-from-exception-and-stack-payloads).

## 12. Guard expensive Rust `log` arguments

The Rust `log` bridge now decides `log::log_enabled!` per target, using that
target's current femtologging level, so a target at `DEBUG` accepts `DEBUG`
records even while another is held at `ERROR`. The `log` crate still evaluates
a macro's arguments for a target held below the process-wide maximum, so add an
explicit `log::log_enabled!` guard around expensive ones. See
[Bridge the log and tracing crates](users-guide.md#bridge-the-log-and-tracing-crates).

## 13. Move application-wide enrichment to the handler

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

## 14. Adopt the new features

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
- [ ] Remove undeclared attributes from `BasicConfig` instances.
- [ ] Attach every handler and filter declared on a `RuntimeConfigBuilder`.
- [ ] Remove any reliance on the timed-rotation test hooks in production builds.
- [ ] Update Rust code for the fallible `FemtoLogger` constructors and
  `add_handler()`.
- [ ] Check each use of `disable_existing_loggers`, including the `fileConfig()`
  default.
- [ ] Move handler state from `threading.local` to `contextvars`.
- [ ] Check the `level` of each handler wrapped in `StdlibHandlerAdapter`.
- [ ] Remove any reliance on partially applied configuration.
- [ ] Flush socket and HTTP handlers before closing them, and check what
  `flush()` returns.
- [ ] Add `log::log_enabled!` guards around expensive Rust `log` arguments.
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
  - **Fix:** Keep the value in your own structure. See
    [task 2](#2-use-only-the-fields-of-basicconfig).
- **Error:**
  `invalid runtime mutation: unreferenced definitions: handlers ["console"] …`
  - **Fix:** See [task 3](#3-attach-everything-a-runtime-change-declares).
- **Error:** `RuntimeError` from `_set_timed_rotation_test_times_for_test`,
  naming the `'test-util' feature`
  - **Fix:** See [task 4](#4-stop-using-the-timed-rotation-test-hooks).
- **Error:** `Python-backed handlers must provide contextual dispatch`
  - **Fix:** Implement contextual dispatch on your Rust handler. See
    [task 5](#5-handle-the-fallible-rust-apis).
- **Error:** `basicConfig() got an unexpected keyword argument 'format'`
  - **Fix:** Build a handler with a `FormatterBuilder` and pass it through
    `handlers`. See [Format output](users-guide.md#format-output).
- **Error:** `… invalid handler configuration: unknown formatter id: plain`
  - **Fix:** Register the formatter with `ConfigBuilder.with_formatter()` or in
    `dictConfig()`'s `formatters` section.
- **Error:** `exception payload graph contains a cycle at $.cause`, or
  `exception payload graph exceeds max_exception_depth=1000`
  - **Fix:** Correct the payload, or pass larger limits. See
    [task 11](#11-check-payloads-passed-to-filter_frames).
- **Warning:**
  `FemtoSocketHandler discarded 12 queued records at its shutdown deadline`, or
  the same from `FemtoHTTPHandler`
  - **Fix:** Call `flush()` before `close()`, or raise the write timeout. See
    [task 10](#10-flush-network-handlers-before-closing-them).
