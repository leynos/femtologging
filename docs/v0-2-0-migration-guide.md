# v0.2.0 migration guide

This guide helps an application that already uses femtologging 0.1 move to
femtologging 0.2.0. Readers who are new to femtologging need none of it: start
with the [users' guide](users-guide.md) instead.

Most of 0.2.0 is new, opt-in capability. A handful of changes can need an edit
before an existing application works as before, so this guide puts those first,
then introduces the new features. An upgrade can stop as soon as the
application works and come back for the rest later. Every Python example here
is run by the project's test suite, so it works as printed.

______________________________________________________________________

## Before you upgrade

Femtologging 0.2.0 is currently published as a beta, and installers skip
pre-releases unless asked. Request it explicitly, and pin the exact beta so
that moving to a later pre-release, which may bring further changes, is a
deliberate choice:

<!-- tested-example: migration-install -->

```bash
python -m pip install "femtologging==0.2.0b1"
```

Then work through Table 1. Each row names a change and the code it affects; a
row that does not match your application can be skipped.

| Change                                                                                              | Affects code that…                                                         |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| [`BasicConfig` is a slotted dataclass](#basicconfig-is-a-slotted-dataclass)                         | sets attributes on a `BasicConfig` beyond its five fields                  |
| [Configuration is transactional](#configuration-is-applied-all-or-nothing)                          | relied on a failed configuration call applying part of its changes         |
| [Python handlers see the caller's context](#python-handlers-run-in-the-callers-contextvars-context) | reads `contextvars` or thread-local state inside a handler                 |
| [HTTP `close()` is bounded](#http-handler-close-no-longer-waits-for-retries)                        | relied on `FemtoHTTPHandler.close()` waiting for every retry               |
| [Rust handler trait changes](#rust-handler-and-logger-api-changes)                                  | implements `FemtoHandlerTrait` or calls `FemtoLogger::add_handler` in Rust |

_Table 1: Upgrade checklist for applications using femtologging 0.1._

______________________________________________________________________

## Breaking changes

### `BasicConfig` is a slotted dataclass

`BasicConfig` is now declared with `@dataclasses.dataclass(slots=True)`. An
instance can hold only its five declared fields: `level`, `filename`, `stream`,
`force`, and `handlers`. A mistyped or invented attribute name now fails at the
point of assignment, instead of silently creating an attribute that
`basicConfig()` never reads.

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

To migrate, use one of the declared fields if the value belongs to
`basicConfig()`. Otherwise keep auxiliary state in your own structure, such as
a separate variable, dataclass, or dictionary. Constructing `BasicConfig` and
calling `basicConfig()` are otherwise unchanged.

### Configuration is applied all or nothing

`ConfigBuilder.build_and_init()` now builds every handler and filter, checks
every reference, and prepares every logger before it changes any live logger.
`dictConfig()` and `fileConfig()` use the same path. A call that raises leaves
the previous configuration exactly as it was:

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

If your start-up code caught a configuration error and carried on, expecting
the valid parts to have been applied, apply a corrected configuration instead.

### Python handlers run in the caller's `contextvars` context

Python handlers, including `StdlibHandlerAdapter`, still run on the logger's
worker thread, but each call now runs inside a copy of the `contextvars`
context that was active when the record was logged. For the adapter, that
covers the wrapped handler's whole `handle()` call: its filters, formatter, and
`emit()`.

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

No code change is needed to benefit. Only `contextvars` travel with the record;
`threading.local` and other thread-local state still belong to the worker
thread. A handler that read worker-thread state through a `ContextVar` will now
see the caller's value instead. Move such state to `contextvars` if it should
follow the caller, or out of the `ContextVar` if it should not.

### HTTP handler `close()` no longer waits for retries

`FemtoHTTPHandler.close()`, and dropping the handler, now wait for the worker
for at most the handler's write timeout. A worker that is still retrying
against an unresponsive endpoint is abandoned with a warning rather than
joined. It keeps sending the records already queued, and those arrive only if
the process lives long enough.

If your shutdown relied on `close()` blocking until every record was delivered,
keep the process alive until the endpoint has accepted them, or call `flush()`
(which waits for the worker without abandoning it) first.

### Rust handler and logger API changes

This affects only Rust code that implements `FemtoHandlerTrait` or attaches
handlers from Rust.

- `FemtoLogger::add_handler()` returns `Result<(), HandlerError>`. Handle or
  propagate the result.
- A handler whose `is_python_backed()` returns `true` must also return `true`
  from `provides_context_dispatch()` and override `handle_with_context()` to
  invoke Python inside the supplied context. Otherwise `add_handler()` fails
  with `HandlerError::MissingContextDispatch`, which reaches Python as
  `RuntimeError("Python-backed handlers must provide contextual dispatch")`.

Handlers attached from Python are unaffected: femtologging wraps them in an
implementation that already provides contextual dispatch.

______________________________________________________________________

## New in 0.2.0

Everything in this part is additive. Existing logging calls keep working
unchanged.

### Structured fields on every log call

`log()`, `debug()`, `info()`, `warning()`, `error()`, and `critical()` accept a
keyword-only `extra` mapping. Values must be `str`, `int`, `float`, `bool`, or
`None`, and an invalid mapping raises instead of being silently discarded.
`exception()` does not accept `extra`.

<!-- tested-example: migration-extra -->

```python
from femtologging import get_logger

logger = get_logger("service")
assert logger.info("request accepted", extra={"request_id": 42, "user": "alice"})
```

See [Attach fields to one record](users-guide.md#attach-fields-to-one-record)
for the limits that apply.

### Scoped context that follows asyncio tasks

`log_context(**fields)` attaches fields to every record logged inside a `with`
block, including through loggers returned by `get_logger()`. It is built on
`contextvars`, so it is safe across `await`: concurrent tasks on one event-loop
thread never see each other's fields, and a child task inherits a snapshot of
its parent's fields at creation. Fields in `extra` override same-named context
fields.

<!-- tested-example: migration-log-context -->

```python
import femtologging

logger = femtologging.get_logger("service")
with femtologging.log_context(request_id=42):
    assert logger.info("request accepted") == "service [INFO] request accepted"
```

Context is captured when the record is created, so leaving the block later does
not change records already logged. Rust scoped context (`push_log_context`)
stays OS-thread-local, and the two never mix; see
[Rust and Python context](users-guide.md#rust-and-python-context).

### Python callback filters

`PythonCallbackFilterBuilder` turns a callable, or an object with a
standard-library-style `filter(record)` method, into a filter. It runs on the
logging thread, and attributes it adds to the record become structured fields.
`dictConfig()` accepts the standard `"()"` factory syntax for such filters. See
[Filter and enrich records](users-guide.md#filter-and-enrich-records).

### Time-based file rotation

`FemtoTimedRotatingFileHandler`, `TimedHandlerOptions`, and
`TimedRotatingFileHandlerBuilder` rotate files on a schedule, and
`dictConfig()` and `fileConfig()` accept
`logging.handlers.TimedRotatingFileHandler`. See
[Rotate files on a schedule](users-guide.md#rotate-files-on-a-schedule).

### Runtime reconfiguration

`RuntimeConfigBuilder` and `LoggerMutationBuilder` replace, append, remove, or
clear handlers and filters on running loggers, and change their levels and
propagation, in one step. See
[Change configuration at runtime](users-guide.md#change-configuration-at-runtime).

### New counters

`FemtoLogger.get_context_dropped()` counts records discarded because Rust
context failed validation, and `get_context_capture_failures()` counts records
whose `contextvars` snapshot could not be taken. Both are separate from
`get_dropped()`, which still counts queue-full drops.

### Reliable flushing

`FemtoLogger.flush_handlers()` and each handler's `flush()` and `close()` now
release the global interpreter lock while they wait. Previously, a call made
straight after logging could stall until its timeout, and report failure,
because the worker needed the lock to deliver the record. `close()` in that
situation could also lose the records still queued.

______________________________________________________________________

## Unchanged

Logger names and the hierarchy, levels, `basicConfig()`, the built-in handlers'
constructors, the `dictConfig()` and `fileConfig()` schemas, and the Python
handler protocol (`handle()` and `handle_record()`) are unchanged. The
[0.1.0 migration guide](v0-1-0-migration-guide.md) still applies to code
written before 0.1.0.
