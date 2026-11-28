# v0.2.0 migration guide

______________________________________________________________________

## Who should read this

This document describes breaking changes and structured logging additions in
v0.2.0, with the steps required to update calling code. Read it when upgrading
to v0.2.0 from v0.1.0, particularly when configuring loggers or file handlers
in Python, or when constructing `FemtoLogger` values in Rust. If you are
upgrading from an earlier release, follow each migration guide in sequence,
starting with the [v0.1.0 guide](v0-1-0-migration-guide.md). Existing logging
calls remain valid; the structured logging APIs add optional keyword arguments
and restore scoped-context propagation through named loggers.

______________________________________________________________________

## Impact at a glance

| Priority           | Area                            | What to do                                                                                                                                                                                       |
| ------------------ | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Required           | Rust logger construction        | Handle the now-fallible `FemtoLogger::new` (`PyResult<Self>`) and `FemtoLogger::with_parent` (`Result<Self, DispatcherError>`).                                                                  |
| Required if called | `BasicConfig`                   | Remove assignments to attributes outside the five declared fields.                                                                                                                               |
| Required if called | `fileConfig` handler parameters | Shorten raw handler `args` or `kwargs` values that exceed 65,536 characters; the limit applies before placeholder expansion.                                                                     |
| Required if called | Logger names                    | Give loggers stable dotted names; invalid shapes and names over the registry cap now raise.                                                                                                      |
| Required if called | Reserved structured-field names | Rename any `extra=`, `log_context(...)`, `push_log_context`, or `RecordMetadata::key_values` name that uses a built-in record name; those names now raise, or the record is dropped and counted. |
| Recommended        | `log_context` and named loggers | Rely on scoped context reaching named loggers and delete workarounds that copied fields into every call.                                                                                         |
| Review if enabled  | Async `log_context` use         | Replace reliance on thread-local leakage between tasks with `contextvars`-aware code.                                                                                                            |
| Review if enabled  | Shared dispatch                 | Review handlers that keep per-logger state in `threading.local`; use `contextvars` or keep the state with the handler.                                                                           |
| Review if enabled  | `StdlibHandlerAdapter`          | Review handler levels and `contextvars`-dependent filters now that dispatch honours both.                                                                                                        |
| Review if enabled  | Handler filters and formatters  | Move shared enrichment to handler filters and registered formatters.                                                                                                                             |
| Review if enabled  | File handlers                   | Add explicit synchronization wherever a file-handler call was relied on to exclude other Python threads.                                                                                         |
| Review if enabled  | `disable_existing_loggers`      | Name every logger that must keep emitting; `disable_existing_loggers=False` preserves but does not re-enable an earlier-disabled logger.                                                         |
| Review if enabled  | `dictConfig` filter factories   | Pass `allow_factories=False` for lower-trust configuration so `"()"` filter entries are rejected before import, resolution or invocation.                                                        |
| Review if enabled  | Socket and HTTP handlers        | Call `flush()` before `close()` and accept that a stalled worker drops queued records at the single shutdown deadline.                                                                           |
| Review if enabled  | HTTP handlers                   | Expect `HandlerConfigError` for an invalid endpoint or header, and check the four-redirect limit and credential stripping.                                                                       |
| Optional           | Inline structured fields        | Adopt `extra=` for per-call structured fields.                                                                                                                                                   |
| Optional           | Exception-payload limits        | Set `max_exception_depth` and `max_exception_nodes` for unbounded payloads.                                                                                                                      |
| Review if enabled  | Capture ceilings                | Review Rust struct-literal payload construction and callback filters that inspect exception or stack data for the new bounds.                                                                    |
| Required for Rust  | `SchemaVersionError` matches    | Add a wildcard arm; the enum is now `#[non_exhaustive]` and gains nested and limit variants.                                                                                                     |
| Optional           | Handler wait diagnostics        | Read `wait_diagnostics()` to tell a dropped record from a slow worker; enable `tracing-compat` for internal wait events.                                                                         |

_Table 1: Obligations when moving to v0.2.0._

______________________________________________________________________

## 1. `BasicConfig` is now a slotted dataclass

`BasicConfig` (exported from `femtologging`) is now declared with
`@dataclasses.dataclass(slots=True)`. Slots remove the per-instance `__dict__`,
so a `BasicConfig` instance can no longer hold attributes beyond its five
declared fields: `level`, `filename`, `stream`, `force`, and `handlers`. The
benefit is that a mistyped or aspirational attribute name now fails loudly with
an `AttributeError` at the point of assignment, instead of silently creating a
dead attribute that the rest of the code never reads.

### What breaks

Assigning any attribute not among the declared fields now raises
`AttributeError` instead of succeeding silently.

### Before

```python
cfg = BasicConfig(level="INFO")
cfg.filename_backup = "app.log.bak"  # silently created a dead attribute
```

### After

```python
cfg = BasicConfig(level="INFO")
# `filename_backup` is not a declared field, so this now raises:
# AttributeError: 'BasicConfig' object has no attribute 'filename_backup'
cfg.filename_backup = "app.log.bak"
```

### How to migrate

- Use one of the declared fields (`level`, `filename`, `stream`, `force`,
  `handlers`) if the value belongs in `basicConfig`'s configuration surface.
- Otherwise, keep auxiliary state in the caller's own structure — a separate
  variable, dataclass, or dictionary — rather than attaching it to the
  `BasicConfig` instance.

______________________________________________________________________

## 2. Inline structured fields

`FemtoLogger.log()`, `debug()`, `info()`, `warning()`, `error()`, and
`critical()` now accept the optional keyword-only argument `extra`:

```python
logger.info(
    "request accepted",
    extra={"request_id": 42, "user": "alice"},
)
```

`extra` must be a mapping with string keys and scalar values of type `str`,
`int`, `float`, `bool`, or `None`. The existing context limits apply: at most
64 keys, 64 UTF-8 bytes per key, 1,024 UTF-8 bytes per value, and 16 KiB of
total serialized fields per record. Invalid mappings or values raise an
exception instead of being silently discarded.

When a field is present both in `extra` and in an active `log_context`, the
inline value takes precedence. `exception()` is unchanged and does not accept
`extra`.

Field names that belong to the record itself are reserved. `name`, `msg`,
`args`, `levelname`, `levelno`, `pathname`, `filename`, `module`, `exc_info`,
`exc_text`, `stack_info`, `lineno`, `funcName`, `created`, `msecs`,
`relativeCreated`, `thread`, `threadName`, `process`, `processName`, `message`,
`asctime`, `taskName`, `logger`, `level`, `metadata`, `module_path`,
`line_number`, `timestamp`, `thread_id`, `thread_name`, and `key_values` all
raise `ValueError` when used as an `extra` key or a `log_context(...)` key.
Matching is exact and case-sensitive, so `Name` and `logger_name` remain
legitimate custom fields. Rust callers see the same contract:
`push_log_context` returns `LogContextError::ReservedKey`, and
`log_with_metadata` drops a record whose context contains a reserved name and
counts it through `get_context_dropped()`. An enabled logger counts that drop
even when the record is below its severity threshold, because context admission
runs before the level gate. A logger that configuration disabled never reaches
admission: it returns first and increments the disabled-logger counter instead.

Renaming the field fixes admission, but an HTTP handler that selected the old
name must be updated with it, because `with_record_fields` matches top-level
wire names.

Before:

```python
logger.info("order accepted", extra={"message": "accepted"})
handler = HTTPHandlerBuilder().with_record_fields(["name", "message"]).build()
```

After:

```python
logger.info("order accepted", extra={"message_id": "accepted"})
handler = HTTPHandlerBuilder().with_record_fields(["name", "message_id"]).build()
```

Selections of authoritative fields such as `msg` need no change: they name the
built-in field rather than a custom one.

______________________________________________________________________

## 3. Scoped context through named loggers

`log_context()` fields now reach records emitted through `FemtoLogger` methods,
including methods on loggers returned by `get_logger()`, while the context is
active in the current Python task:

```python
logger = femtologging.get_logger("service")
with femtologging.log_context(request_id=42):
    logger.info("request accepted")
```

The context follows the current Python task across `await` points. Use the
per-call `extra` argument when metadata belongs to one operation but should not
be inherited by child tasks.

## 4. Scoped logging context follows Python tasks

Python `log_context(**fields)` used to store fields in an OS-thread-local
stack. In asynchronous code, tasks sharing an event-loop thread could observe
each other's active fields while one task was suspended. The context manager
now uses `contextvars.ContextVar`, so its fields follow the current Python
context. A newly created child task inherits a snapshot of the parent's context
at task creation; later changes in either task do not change the other's
context. Nested `log_context` scopes override same-named fields from outer
scopes.

Python logging methods and module-level logging functions copy the active
Python context when they create a record, before it is queued. Changing or
leaving the context after that call does not alter the record. For Rust
records, explicit key-values supplied by a macro or tracing event take
precedence over same-named Rust scoped fields.

Rust scoped context remains OS-thread-local. Its guard is `!Send` and `!Sync`,
and removes its own frame when dropped on the thread that created it. Keep a
Rust guard within a thread-affine synchronous scope; do not hold it across an
`await` that may resume on another thread. Rust macros and the Rust `log` and
`tracing` bridges merge Rust scoped fields when they create a record, with the
tracing bridge also retaining event and span fields. Explicit event fields take
precedence over same-named scoped fields. Python task-local fields and Rust
thread-local fields do not transfer implicitly in either direction. Pass
metadata explicitly when a request crosses that boundary.

______________________________________________________________________

## 5. Bounded logger names and shared dispatch

Logger names now have explicit shape and capacity limits. A name must be
non-empty, must not start or end with `.`, and must not contain consecutive
dots. It is limited to 255 UTF-8 bytes and 32 dot-separated segments, and may
not contain control characters or the Unicode line and paragraph separators.
Invalid names raise `ValueError` before they become registry keys.

The process-wide registry holds at most 10,000 named loggers. The root logger
is exempt; names already registered keep their identity, and the manager never
evicts a logger to make room. A new name whose full parent chain would exceed
the cap raises `RuntimeError`. Names used by the Rust `log` and `tracing`
bridges fall back to the root logger when validation or the registry cap
refuses them. The `log` bridge emits a rate-limited diagnostic; the tracing
layer currently falls back silently.

Use stable component names such as `service.database`, and put request, user,
or tenant identifiers in structured fields instead. A one-logger-per-request
pattern now reaches the cap and is refused. See the
[users' guide](users-guide.md#creating-and-naming-loggers) for examples.

Loggers no longer own worker threads. A process lazily starts a fixed pool of
one to four dispatch workers, based on available parallelism, and assigns
loggers to their workers round-robin. Loggers sharing a shard also share its
worker; per-logger FIFO order is preserved. Code that relied on a custom
handler's `threading.local` state being private to one logger must use
`contextvars` for producer context or keep that state explicitly with the
handler. Python `contextvars` are captured before dispatch and restored while
the handler runs.

If the operating system cannot start the pool because the process has reached
its thread limit, logger creation raises `RuntimeError` instead of panicking. A
later logger creation retries pool startup.

Rust callers must handle the same failures as values, because both constructors
are now fallible. `FemtoLogger::new` returns `PyResult<Self>` and rejects an
invalid name; Python callers see that refusal as `ValueError`.
`FemtoLogger::with_parent` returns `Result<Self, DispatcherError>` and reports
a pool that could not start, which Python callers see as `RuntimeError`.
`DispatcherError` is re-exported from the crate root, so both can be matched
without naming the module path. Code that used either result directly as a
`FemtoLogger` no longer compiles. A failed pool startup is retried by the next
logger creation.

______________________________________________________________________

## `FemtoLogger.flush_handlers()` releases the GIL and flushes Python handlers

`FemtoLogger.flush_handlers()` now releases the GIL for the whole operation,
not only the wait for the logger's dispatch shard to drain. The barrier still
waits behind the records already queued for that logger, but the attached
handlers are then flushed while the caller is detached from the interpreter. A
Python formatter queued on a native handler's worker thread, or a Python
handler's own `flush()`, can therefore run instead of starving behind the
thread that called the flush.

Three observable behaviours change:

- **Python handler `flush()` methods run.** A handler object that is not a
  built-in femtologging handler is now asked to flush through its callable
  `flush` attribute. Previously the flush stopped at a default no-op, so a
  custom handler that buffered data never saw the call. A handler with no
  `flush`, or with a non-callable one, still succeeds silently.
- **Python handler failures reach the caller.** `flush_handlers()` returns
  `False` when a Python handler's `flush()` raises, returns a falsy value other
  than `None`, or fails during attribute lookup for a reason other than a
  missing or non-callable `flush`. `logging.Handler.flush()` returns `None`, so
  a well-behaved standard handler still counts as success.
- **A failed flush is visible in the Rust bridge.** When `log::Log::flush`
  sweeps the registered loggers, a failed logger flush now emits a rate-limited
  warning instead of being discarded, and the attempt is also counted in
  process-wide counters. `log`'s trait still returns nothing, so the warning is
  the only evidence an operator can read today; the counters are not yet
  exported. [Issue 551](https://github.com/leynos/femtologging/issues/551) owns
  the telemetry interface, the exporter, and the alerting documentation.

Shutdown order matters more than before. A flush is a barrier over the records
queued when it runs; a producer that logs afterwards can leave a record queued
even though the flush returned `True`. Stop and join every logging producer
before flushing loggers and closing their handlers.

No method signature changed. A caller that already treats `flush_handlers()` as
a best-effort barrier guarded by its boolean result needs no change.

______________________________________________________________________

## Handler `close()` is bounded end to end

`FemtoSocketHandler.close()` and `FemtoHTTPHandler.close()` now run on one
deadline, taken from the handler's write timeout, that covers the whole
shutdown wait: signalling the worker, queueing the acknowledgement request,
waiting for the acknowledgement, and joining the worker. Previously the
acknowledgement wait and the join each had their own or no bound, so a worker
stuck in a blocking network call could hold `close()` for far longer than the
timeout suggested. `Drop` calls `close()`, so the same bound applies to a
handler that is simply dropped.

A worker that does not acknowledge within the budget is abandoned rather than
joined. Until the deadline, a worker draining its queue makes one best-effort
attempt per record; once `close()` abandons it, the signal is aborted, so the
worker finishes only the network call it was already in, discards whatever is
still queued, and exits on its own with nothing left to join it. This changes
three observable behaviours:

- **The shutdown wait does not run past the budget.** Code that relied on
  `close()` blocking until every queued record had been sent must call
  `flush()` first, and must accept that a flush is best-effort under the same
  timeout. A pending drop report runs before the wait and is not covered by the
  budget, so a blocking log sink can still delay the call.
- **Records can be dropped at the deadline.** Whatever is still queued when the
  drain deadline passes is discarded rather than sent, and one rate-limited
  warning reports how many records went that way. A collector that cannot keep
  up with the shutdown budget now loses records instead of extending shutdown.
- **A partially written frame stays partially written.** A frame that runs out
  of budget mid-write reports how many of its bytes were sent. The connection
  is closed rather than retried, so the collector may observe a truncated frame.
- **A socket `flush()` is bounded by one budget, not two.** Its flush timeout
  is now a single deadline shared by the wait for queue space and the
  acknowledgement. Previously each was given the full timeout, so a flush could
  take up to twice the configured value before returning `False`. It now returns
  `False` once that one budget is spent. Treat the return as "the worker
  answered within one flush timeout", not "the worker had a timeout of its own
  to answer". `FemtoHTTPHandler.flush()` already shared one deadline and is
  unchanged.

The stream, file, and rotating-file handler flushes use the same accounting. A
direct `flush()` on any worker-queue handler now spends one configured flush
timeout across queue admission and the acknowledgement, so the acknowledgement
may receive less than the full timeout, and a busy worker can make `flush()`
return `False` sooner than before. The logger's `flush_handlers()` does not
change this: it gives the handler the barrier's deadline, and the handler still
caps its wait at its own configured timeout.

The bound on the shutdown wait is the budget: a worker that misses the deadline
is detached, so that wait never runs past it. The bound on the worker's own
exit is looser, because it is on what the worker _starts_: no standard-library
API cancels a blocking `connect`, `write`, or TLS handshake, so the worker may
live on for up to one network call already in flight when the deadline fell.
Nothing new starts after the deadline.

No API changed. The contract is unchanged for callers that already treat
`close()` as best-effort, and `flush()` keeps its existing boolean return.

______________________________________________________________________

## Dropped-record warning messages are cumulative

The handler drop warnings now count every drop since the previous report
instead of only the drops in one rate-limit interval, and a flush, close or
destruction reports a count that is still pending.

| Component | Before                                                                    | After                                                                                                                                                                                                                    |
| --------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Stream    | `FemtoStreamHandler: {count} log records dropped in the last interval`    | `FemtoStreamHandler: {count} log records dropped since the last report`                                                                                                                                                  |
| HTTP      | `FemtoHTTPHandler dropped {count} records in the last interval`           | `FemtoHTTPHandler dropped {count} records since the last report`, with `; triggering cause: {cause}` appended when a specific drop triggered the report                                                                  |
| Socket    | `FemtoSocketHandler dropped {count} records in the last interval`         | `FemtoSocketHandler dropped {count} records since the last report`, with `; triggering cause: {cause}` appended when a specific drop triggered the report                                                                |
| Logger    | No destruction report; a suppressed count waited for the next due warning | `FemtoLogger: dropped {count} records; reported by flush` for dispatch-admission drops, and `FemtoLogger: dropped {count} records after context capture failed` for context-capture drops, emitted at logger destruction |

The old wording also had a separate sentence per failure cause — for example
`FemtoHTTPHandler queue full; dropped {count} records` or
`FemtoSocketHandler dropped {count} records due to write errors`. Those are
replaced by the single cumulative form plus the `triggering cause` suffix, so
code matching a cause-specific sentence must match the new form instead.

A suppressed interval now carries its drops into the next report, so one report
can cover several intervals. The count is what the reporting path took when it
claimed it, so a drop recorded after that take belongs to the next report, and
a count another caller has already taken is not included even if that caller
has not reported it yet. The `triggering cause` label names only the cause that
triggered the report; the count may include earlier drops from other causes.

An explicit `flush()` on an open handler reports pending counts inside the
interval, including the worker's own suppressed count, but it carries them on a
marker, so a marker that never enters a queue leaves the count pending for the
next report. A flush on a handler that has already closed reports nothing and
returns `false`, because that call is deadline-bound; the count it refused
after close is reported by the next `close()` or by destruction. For the HTTP
and socket handlers the close-time report runs before the bounded shutdown wait
and is outside `flush_timeout`, so a blocking log sink can delay `close()` past
its budget.

______________________________________________________________________

## 6. `StdlibHandlerAdapter` honours the wrapped handler level

`StdlibHandlerAdapter` now checks two thresholds in order. `FemtoLogger` first
checks its own level before dispatching a record; the adapter then checks the
wrapped standard-library handler's current `level`. Records below the handler
level are discarded before `Handler.handle()` runs, so that handler's filters
and emission path do not run for them. `logging.NOTSET` remains permissive and
accepts every mapped level.

The adapter reads the handler level when it processes each queued record. A
call to `handler.setLevel()` therefore affects records processed after the
change, including queued records not yet handled. There is no need to rebuild
the adapter. For thresholds and configuration examples, see the
[stdlib handler guide](users-guide.md#using-standard-library-stdlib-logginghandler-subclasses).

______________________________________________________________________

## 7. `StdlibHandlerAdapter` propagates `contextvars`

When a record is sent through `StdlibHandlerAdapter`, femtologging captures the
producer's active `contextvars.Context` before queueing the record. On the
logger's shared dispatch-shard worker, `Context.run` invokes the wrapped stdlib
handler's `handle()` method inside that context. With base `Handler.handle()`
behaviour, the filter chain and `emit()` invocation run synchronously in the
captured context, as does formatting performed during that call. Dispatch
remains asynchronous. A handler that defers formatting or emission to another
thread or process needs separate context propagation. Inline filters and
formatters that read a `contextvars.ContextVar` therefore observe the value
present in the producer's captured context.

No code changes are required for this behaviour. Only `contextvars` are
propagated; other thread-local state, including `threading.local`, belongs to
the shared shard worker and may be observed by handlers serving different
loggers. Update filters that depend on such state to use `contextvars` when
they need values from the producer's captured context.

______________________________________________________________________

## The Rust `log` bridge enablement decision is target-aware

The Rust `log` bridge no longer decides enablement from a single process-wide
threshold alone. `log::log_enabled!` now asks two questions: whether the
process-wide maximum admits the level, and whether the record's own target
accepts it. A target that is registered at `DEBUG` accepts `DEBUG` records even
while another target is held at `ERROR`.

Targets are resolved against the femtologging registry:

- Rust's `module::path` form is normalized to a dotted femtologging name, so
  `service::worker` is matched as `service.worker`.
- A registered target uses its own current level. Levels are not inherited from
  parent loggers.
- A valid but unregistered target uses `INFO`.
- An invalid target uses the registered `root` logger's level, or `INFO` when
  no root logger is registered.

Configuration and level changes apply to later enablement checks, without
re-installing the bridge.

The process-wide maximum remains a conservative prefilter. It is recomputed
after logger level and registry changes, and it is never restrictive enough to
suppress a record the bridge could dispatch. Because the `log` crate checks
that maximum before evaluating a macro call's arguments, and asks the target
check only afterwards, an argument for a target held below the maximum is still
evaluated. Add an explicit `log::log_enabled!` guard when an argument is
expensive. See the [`log` facade bridge](users-guide.md#rust-log-facade-bridge)
in the users' guide for the complete behaviour.

______________________________________________________________________

## 8. Handler filters and formatter builders

Handler builders now accept filter IDs with `with_filters([...])`. Register
those IDs with `ConfigBuilder`, or declare them in the top-level `filters`
section when using `dictConfig`; handler `filters` lists can then refer to
them. The filter chain runs on the producer thread before enqueueing and also
runs for records propagated from child loggers to an ancestor handler. This
makes a shared handler a single attach point for cross-cutting enrichment.
Logger filters remain local to the logger that received the call. For the
configured attachment order and per-handler snapshot semantics, see the
accepted handler-filter addendum in
[ADR-0003](adrs/adr-0003-python-stdlib-filter-parity.md#accepted-addendum-2026-10-14-handler-filters).

Use a `FormatterBuilder` directly when configuring a handler, or register one
with `ConfigBuilder.with_formatter(id, formatter)` and give the handler that ID.
`dictConfig` accepts percent-style `format` and `datefmt` entries in its
`formatters` section, which handlers select by registered ID. The percent
formatter can render standard record fields and structured keys in
`metadata.key_values`; callback-filter enrichment is available to it because
filters run before records are queued.

For migration, move application-wide enrichment from individual logger filters
to a filter attached to the shared handler. For output formatting, replace
unsupported `basicConfig(format=..., datefmt=...)` arguments with a handler
builder and either a direct `FormatterBuilder` or a registered formatter.

`dictConfig` also gains a keyword-only `allow_factories` option. It defaults to
`True`, which keeps the standard library's `"()"` factory form working. Passing
`allow_factories=False` rejects every filter entry containing `"()"` before the
target is imported, resolved or invoked, while declarative `level` and `name`
filters keep working. A `"()"` entry is trusted-code-equivalent: a dotted
string imports a module and calls the resolved object, and a direct callable is
called as given. Pass `allow_factories=False` wherever the configuration comes
from tenants, HTTP APIs, remote configuration services, or writable files, and
validate handler classes and destinations separately, because the option is not
a sandbox. See the users' guide section on the
[filter factory trust boundary](users-guide.md#filter-factory-trust-boundary)
for the full guidance, including which factories run when a live
reconfiguration is applied.

______________________________________________________________________

## `flush_handlers()` reaches the handler

`FemtoLogger.flush_handlers()` now flushes each registered handler rather than
only waiting for the dispatch worker to accept the queued records. A built-in
handler is flushed through its own native implementation; a custom handler that
defines `flush` now receives a call it did not receive before, and can use it
to persist buffered records. A missing or non-callable `flush` still counts as
success, so an existing handler that does not define one needs no change. The
call queues a barrier behind the logger's already queued records and blocks the
thread that invoked `flush_handlers()` while the worker handles those records
and then runs the handler flushes, up to the flush deadline; the handler
callbacks themselves run on the dispatch worker rather than on that invoking
thread. An upgrading handler that depended on caller-thread state, such as a
thread-local or an enclosing interpreter frame, must not do so: the callback
has neither. Handlers are attempted while budget remains; a handler that fails
does not stop the ones after it, but handlers left when the deadline expires
are not flushed, and the caller can return `False` while a callback is still
running. A `flush` that returns `None` succeeds, and every other result
succeeds only when it is truthy. An exception raised by `flush` logs a bounded
warning that omits the exception text and makes `flush_handlers()` return
`False`, so keep the method non-blocking and defensive, as with `handle`.

______________________________________________________________________

## 9. Bounded exception-payload filtering

`filter_frames()` keeps its existing call pattern and adds optional keyword-only
`max_exception_depth` and `max_exception_nodes` limits. Existing calls remain
source-compatible. The defaults are 1,000 exception-graph edges below the root
and 10,000 payload occurrences, including the root.

Externally supplied or mutated exception payloads that contain cycles now raise
`ValueError` with the cycle path. Malformed frame data or exception edges raise
`TypeError`. A legitimate graph deeper than the default depth or larger than
the node budget also raises `ValueError`; callers that intentionally pass such
graphs can set larger limits or sanitize the payload before filtering. Negative
or oversized integer limits raise `OverflowError`. Stack payloads continue to
use the existing frame filters and ignore exception-graph limits.

For the complete parameter and error contract, see
[the users guide section on frame filtering][user-filtering].

[user-filtering]: users-guide.md#filtering-frames-from-python-payloads
[user-flushing]: users-guide.md#managing-handlers-and-custom-sinks
[user-capture]: users-guide.md#bounded-exception-and-stack-capture

______________________________________________________________________

## 10. Bounded exception and stack capture

Exception and stack capture now runs under fixed ceilings. The defaults are a
4,096-byte message, 32 arguments and notes (512 bytes each), 64 frames,
512-byte filenames and source lines, 256-byte function names, 32 cause or
context links, 10 exception-group levels and 15 members, 256 exception nodes,
and 256 KiB of retained text. See
[bounded exception and stack capture][user-capture] for the full contract and
the optional `truncation` mapping.

`ExceptionPayload` and `StackTracePayload` gained an optional `truncation`
field, and `PayloadTruncation` is exported from the crate root. Rust code that
builds either payload with a struct literal must set `truncation` or use
`..Default::default()`.

Callback filters now see the bounded payload after the leading native filter
prefix accepts. Only filters that read exception or stack data need to report
that through `FemtoFilter::inspects_traceback`; level and name filters do not.
Filter order is unchanged.

______________________________________________________________________

## Deadline-aware Rust handler flushing

`FemtoLogger::flush_handlers()` now applies one two-second deadline to marker
insertion, the non-empty handler snapshot, worker acknowledgement and every
handler flush. It returns `false` when any part of that barrier is unconfirmed.
Built-in handlers keep their shorter configured timeouts.

Custom Rust handlers that implement only synchronous `flush()` now fail closed:
the barrier does not call that method and returns `false`. Override
`flush_until(context)` to opt in. The implementation must use bounded waits
against the supplied absolute deadline and report `false` when completion is
not confirmed. The logger cannot interrupt arbitrary synchronous handler code
once it starts; calling an unbounded `flush()` from `flush_until()` can still
occupy the shared worker after the caller returns. In normal builds, logger
drop waits only for that logger's in-flight records, up to the bounded drain
timeout; it does not join the process-wide worker. Loom models join their
logger-owned worker during cleanup.

A user-defined Python handler is asked to flush itself: the barrier calls its
callable `flush` method. A handler that defines no callable `flush` opts out
and is confirmed once the record reaches it; one whose `flush` raises or
returns a falsy value other than `None` reports failure, and the exception text
is never printed in case it quotes a record. `None` counts as success, as
Python's flush protocol reports success that way. The call runs synchronously
on the worker, so the barrier deadline bounds the wait for the answer, not the
duration of the handler's own `flush`.

Python-facing calls release the GIL while waiting for the worker. CPython must
reacquire the interpreter before returning to Python, so interpreter contention
can delay the Python return beyond the Rust barrier deadline; a late handoff
changes the result to `false`.

`FemtoLogger.get_flush_diagnostics()` exposes fixed-cardinality counters for
barrier attempts, confirmations, failure stages and elapsed-time buckets. Those
buckets are disjoint ranges (up to 10 ms, over 10 to 100 ms, over 100 to 500
ms, over 500 ms to 2 s, and over 2 s), so each attempt increments exactly one
rather than a cumulative series. The keys are stable and contain no logger
names, paths, messages or payloads. Values are a best-effort snapshot read from
relaxed atomics, so concurrent updates may appear at different instants.
Exporting them through a shared metrics adapter remains the application's
responsibility. See [the `get_flush_diagnostics()` counters][user-flushing] for
the counter contract.

See the [user guide](users-guide.md#flushing-logger-handlers), the
[developer guide](developers-guide.md#logger-handler-flush-barrier), and the
[Rust handler port guide](formatters-and-handlers-rust-port.md#femtohandler-trait-and-implementations)
for the public contract and implementation details.

______________________________________________________________________

## Recursive schema version validation

`ExceptionPayload::validate_version()` now validates the whole payload tree
rather than only the root. It checks the root, its `cause`, its `context`, and
every exception-group member, depth-first in pre-order. Existing calls need no
source change, but a payload that previously validated may now fail because a
nested payload carries an unsupported `schema_version`.

A nested mismatch is reported as `SchemaVersionError::Nested`, which carries a
dot-separated path such as `cause.exceptions[3].context`. The walk is bounded at
`MAX_EXCEPTION_PAYLOAD_DEPTH` (1,000 edges below the root) and
`MAX_EXCEPTION_PAYLOAD_NODES` (10,000 payload occurrences), matching the
defaults of bounded exception-payload filtering; exceeding either bound reports
`TreeTooDeep` or `TreeTooLarge`.

`SchemaVersionError` gained the `Nested`, `TreeTooDeep`, and `TreeTooLarge`
variants and is now `#[non_exhaustive]`. This is a breaking change for Rust
consumers that matched `VersionTooNew` and `VersionTooOld` exhaustively: add a
wildcard arm. The root-level variants and their fields are unchanged.

______________________________________________________________________

## 10. File-handler calls release the interpreter lock

Explicit Python calls to `handle()`, `flush()`, and `close()` on the file,
rotating-file, and timed-rotating-file handlers now release the interpreter
lock for the whole of any wait on queue capacity, on the flush acknowledgement,
or on worker shutdown. Another Python thread keeps running while one thread is
parked. No call signature changes; the difference is that code which
accidentally relied on such a call excluding other threads must now provide its
own synchronization.

The behaviour that stays the same is the one that makes this safe to adopt:

- `"block"` still waits for room for as long as the worker takes to make it,
  which may be forever if the worker is stuck. Prefer `"timeout:N"` where a
  bounded wait is acceptable; it drops the record when the deadline expires and
  raises `RuntimeError` from the file and rotating bindings, or `ValueError`
  from the timed-rotating binding.
- A `close()` that is already in progress releases its Python borrow before it
  waits, so calls made during the wait reach a handler that is already closed:
  `handle()` raises `RuntimeError`, `flush()` returns `False`, and a second
  `close()` shares the same completion wait. The timed-rotating binding reports
  that closed handler from `handle()` as `ValueError`. A `handle()` or
  `flush()` call that is still waiting holds its shared borrow for the whole
  wait, but a concurrent `close()` needs only a shared borrow too, so it does
  not raise; it takes the shutdown state and waits on the same completion
  signal, bounded by its one-second deadline.
- The handler destructor still performs shutdown with the interpreter lock
  held. Call `close()` explicitly during shutdown to get the detached wait.

For the full contract and the concurrency scenarios that pin it, see
"Concurrency and the GIL for file handlers" in the user guide.

### Observing the detached waits

A wait that no longer holds the interpreter lock is also harder to see from
outside a stack trace, so each of the three handlers now exposes bounded
observability for it:

- `handler.wait_diagnostics()` returns a dictionary with a fixed key set: one
  counter per outcome for `handle()`, `flush()`, and `close()`, the summed wait
  `total_wait_micros`, the longest single wait `max_wait_micros`, and the
  `waits_in_flight` gauge, which is non-zero while a caller is still waiting.
  Reading it takes no worker command and records no wait, so a monitoring loop
  cannot inflate the numbers it reports. It never carries logger names, paths,
  messages, or raw timeout values.
- With the `tracing-compat` feature, each wait also emits one
  `femtologging::internal` trace event carrying `operation`, `handler_kind`,
  `policy_bucket`, `wait_outcome`, and `elapsed_us`. The `femtologging` tracing
  bridge ignores that target, so enabling the events cannot feed wait outcomes
  back into the logs being written.

Both surfaces are additive: no existing signature changes, and nothing must be
configured to keep the current logging behaviour. They exist so an operator can
distinguish a dropped record from a slow worker, and a settled close from one
that timed out, without inspecting the process. See "Handler wait diagnostics"
in the [developers' guide](developers-guide.md#handler-wait-diagnostics) and
"Concurrency and the GIL for file handlers" in the
[users' guide](users-guide.md#concurrency-and-the-gil-for-file-handlers).

## 11. Handler parameter values are size-limited

`fileConfig` now expands `%(name)s` placeholders with a forward-only scanner
and rejects a raw handler `args` or `kwargs` value longer than 65,536 Unicode
code points. The scan is linear in the raw option text; the limit bounds that
scan, not the expanded result.

### What breaks for handler parameters

A handler `args` or `kwargs` value longer than 65,536 characters, as written in
the INI file, raises `ValueError` before any configuration is applied. The
error names the handler, the field, the actual length and the limit, and does
not echo the value. Previously such a value passed through the placeholder
search and on to `dictConfig`.

### Before: oversized raw handler parameter

```ini
[handler_h]
class = femtologging.FileHandler
# A 65,532-character path makes the raw args text 65,537 characters.
args = (<65,532-character string>,)
```

This now raises
`ValueError: handler 'h' args has 65537 characters; the limit is 65536` before
any logger configuration changes.

### After: shortened raw handler parameter

```ini
[handler_h]
class = femtologging.FileHandler
args = ('/var/log/app.log',)
```

The shortened raw text loads normally. If the value is still large after
substitution, the limit does not apply to the expanded result.

### How to migrate handler parameters

Shorten the raw `args` or `kwargs` text so it fits within 65,536 characters.
The limit applies before substitution, so a short raw parameter may still
expand through a large `[DEFAULT]` value; that expanded size is not capped
here. Substitution remains a single pass: a default value that contains
`%(name)s` is inserted verbatim and is not expanded again.

## Production wheels omit the timed-rotation test hooks

The persistent `[tool.maturin]` feature set is now `["python"]` rather than
`["test-util"]`. Wheels and the source distribution therefore no longer export
`set_timed_rotation_test_times_for_test` or
`clear_timed_rotation_test_times_for_test`, and `femtologging._has_test_util` is
`False`. Their underscore-prefixed Python wrappers,
`_set_timed_rotation_test_times_for_test` and
`_clear_timed_rotation_test_times_for_test`, are no longer re-exported by the
package in any build, and a production build does not define them, or the other
`*_for_test` wrappers, in the private `femtologging._rust_compat` module
either. Test code imports them from that module in a `test-util` build.

These helpers were never supported runtime API. Code that relied on them must
build the extension with the `test-util` feature, either by setting
`UV_CONFIG_FILE=.github/uv-test-util.toml` for a source install or by passing
`--features python,test-util` to a development build. The rotating
fresh-failure helpers `_force_rotating_fresh_failure_for_test` and
`_clear_rotating_fresh_failure_for_test` follow the same rule: neither the
package nor the private compatibility module defines them without `test-util`.
They are test-only and carry no support guarantee.

The section
[Test-only Python and Rust hooks leave the default API](#test-only-python-and-rust-hooks-leave-the-default-api)
covers the rest of the surface that left the default build.

## Test-only Python and Rust hooks leave the default API

The default extension surface no longer contains helpers that only tests use.
Each removal below is a deliberate breaking change, and each lists the
migration step.

### Handler and filter addresses are gone

`FemtoLogger.handler_ptrs_for_test()` and `FemtoLogger.filter_ptrs_for_test()`
are removed. They returned native addresses, which leaked process layout across
the Python boundary.

Test builds now expose `handler_identities_for_test()` and
`filter_identities_for_test()`. They return opaque `AttachmentIdentity` tokens
in attachment order. The tokens compare by allocation identity, never expose an
address, and are not registered with the Python module.

Migration step: compare tokens instead of addresses. For example, capture
`before = logger.handler_identities_for_test()` before a reconfiguration and
compare it with a later read. Do not unwrap a token to an integer; the token
has no integer or index conversion.

### The Rust emit and install hooks are gated

`_emit_rust_log`, `_rust_log_enabled`, `_rust_log_max_level`,
`_emit_rust_tracing_event`, `_emit_rust_tracing_structured_event`,
`_emit_rust_tracing_span_event`, `_install_test_global_rust_logger` and
`_install_test_global_tracing_subscriber` are absent from production wheels and
from the shipped `.pyi` stub.

Migration step: no runtime configuration can restore them. Build the extension
with `test-util` and the matching `log-compat` or `tracing-compat` feature when
a test needs them, then bind them defensively (for example with `getattr`) so
the same test module also collects against a production wheel.

### The Rust test seams require `test-util`

`TestConfig` and `FemtoFileHandler::with_writer_for_test()` are compiled only
for unit tests and `test-util` builds, together with their crate-root
re-exports.

Migration step: a default-feature Rust consumer that used them must enable
`test-util` explicitly. They are test-only and carry no support guarantee.

______________________________________________________________________

## `disable_existing_loggers` disables omitted loggers

A configuration with `disable_existing_loggers` enabled now flags every omitted
logger as disabled instead of only clearing its handlers. The difference is
observable: a disabled logger rejects records in its own producer path, so its
`log_record` and `log` return `None` before the level check, filters, the
logger formatter, and dispatch run, and its records never propagate to ancestor
handlers. Such a logger previously still forwarded records to ancestor handlers
after losing its own. `isEnabledFor` reports `False` for every level on a
disabled logger, and each refusal increments `logger.get_disabled_drops()`, so
configuration-disabled traffic is visible separately from queue-pressure drops.

Naming a disabled logger in a later configuration re-enables it, and a
configuration with `disable_existing_loggers=False` leaves the existing
disabled flag untouched. Ancestors of configured loggers are kept with their
attachments, so a configured descendant can still propagate to them, but
keep-set membership does not change their disabled state: an ancestor that was
already disabled stays disabled. Only a logger the configuration names
explicitly is re-enabled.

The configuration itself is now all-or-nothing for live state: a rejected
configuration leaves every pre-existing logger as it was. Two side effects sit
outside that guarantee. Records already queued or snapshotted by a handler
before the commit are not retracted, and handler construction during
preparation can open a file or socket before a later preparation failure
rejects the configuration.

### What breaks for omitted loggers

Loggers omitted from a new configuration stop emitting. Calls on them return
`None`, no filter, formatter, or handler runs, and nothing reaches an ancestor
handler through propagation. Code that relied on an omitted logger continuing
to reach a handler on an ancestor, or on `isEnabledFor` still reporting its
configured level, now sees no output.

### Before: an omitted logger still reached root

```python
# Create the logger before configuration, or the configuration never sees it.
noisy = femtologging.get_logger("noisy")
femtologging.dictConfig({
    "version": 1,
    "disable_existing_loggers": True,
    "handlers": {"console": {"class": "logging.StreamHandler"}},
    "root": {"handlers": ["console"], "level": "ERROR"},
})
# The omitted logger was not disabled, so it still propagated to `root`.
noisy.error("still seen")  # reached root
```

### After: an omitted logger is disabled

```python
# Create the logger before configuration so the configuration disables it.
noisy = femtologging.get_logger("noisy")
femtologging.dictConfig({
    "version": 1,
    "disable_existing_loggers": True,
    "handlers": {"console": {"class": "logging.StreamHandler"}},
    "root": {"handlers": ["console"], "level": "ERROR"},
})
# The omitted logger is now disabled: this returns None and reaches nothing.
noisy.error("dropped")
```

### How to migrate omitted loggers

- Name every logger that must keep emitting in the new configuration. An
  explicit entry re-enables that logger. Its kept ancestors retain their
  existing enabled or disabled state and their attachments.
- Set `disable_existing_loggers=False` when existing loggers should keep their
  handlers, filters, and enabled state. It preserves a disabled flag rather
  than clearing it, so a logger an earlier configuration disabled stays
  disabled; name that logger explicitly to re-enable it.
- Move diagnostic calls to a configured logger, or add the emitting logger to
  the configuration, instead of relying on propagation from an omitted logger.
- Do not rely on a configuration rollback to retract records already queued or
  to undo a file or socket opened during handler construction.

______________________________________________________________________

## Stream handler flush deadline and detached close

`FemtoStreamHandler.flush()` now applies its configured timeout once. The
deadline covers both queueing the flush command and waiting for the worker's
acknowledgement; previously each wait received the full timeout, so a call
could take nearly twice the configured limit. `close()` now waits for the
worker's completion signal and detaches the thread instead of joining it, so a
blocking thread-local destructor can no longer hold `close()` past its timeout.

### What breaks for stream handler shutdown

- A `flush()` that previously succeeded because the acknowledgement arrived
  after the first timeout but within a second one now returns `False`.
- `close()` can return while the worker thread is still running thread-local
  destructors. Code that assumed the thread had exited when `close()` returned
  must not rely on that.

### How to migrate stream handler shutdown

- `flush()` returns `False` for four distinct reasons: the shared deadline
  expired while queueing the command or while waiting for the acknowledgement,
  the handler was already closed, the channel to the worker had been
  disconnected, or the worker reported an I/O error. The return value does not
  say which, so a caller cannot tell a disconnection from a timeout by the
  result alone. Enable `trace` logging for the `FemtoStreamHandler` flush
  messages to identify the failing stage and cause — queueing versus
  acknowledgement, and a timeout versus a disconnection — before deciding
  whether to retry. The fixed messages are documented in the
  [Rust log bridge and handler waits][dev-log-bridge] section. Only a timeout
  is worth a bounded retry or a larger
  `StreamHandlerBuilder.with_flush_after_ms()`. A closed handler must be
  rebuilt, and an I/O error needs the underlying stream repaired.
- `close()` returns `None` whether or not it observed the worker's completion
  signal, so there is no return value to check. Code that must know the worker
  finished needs a signal the application owns.
- Do not use `close()` as a barrier for thread-local teardown. If teardown must
  finish before process exit, perform it in the formatter's own code before the
  worker body returns, or synchronize on a signal the application owns.

[dev-log-bridge]: developers-guide.md#rust-log-bridge-and-handler-waits

______________________________________________________________________

## HTTP handler endpoint validation and redirects

`HTTPHandlerBuilder` now validates what the HTTP transport will send, and the
worker applies ureq 2's redirect rules itself instead of delegating them to the
HTTP client.

### What changes

- An endpoint that is not an absolute `http` or `https` URL, or that the
  transport cannot express, raises `HandlerConfigError` when the handler is
  built. It no longer builds and then fails on every record.
- A header name, header value, or bearer token that cannot be encoded as an
  HTTP header raises `HandlerConfigError` at build time. The message does not
  echo the value.
- A path that needs percent-encoding is still accepted and encoded: a space in
  the path is sent as `%20`. A GET payload is appended to the configured
  endpoint before the endpoint is parsed, so an endpoint ending in a space
  still sends the encoded form followed by the query.
- At most four redirects are followed. A 300, a 307 or 308 answering a POST, and
  a redirect with a missing or non-UTF-8 `Location` are permanent outcomes. A
  fifth redirect is retried as a transport failure, `Authorization` and
  `Cookie` are removed before the redirect target is requested, and a
  `Location` with a space is followed to its percent-encoded form.
- `write_timeout` also bounds connection establishment, so a connection waits
  for at most the smaller of `connect_timeout` and `write_timeout`.
- A whole redirect chain shares one `write_timeout` budget. A chain that is
  still going when the budget expires fails as a transport error instead of
  spending the timeout again on each hop.
- A transport failure now logs one fixed classification, such as
  `transport error (invalid URL)` or `transport error (DNS lookup failed)`,
  instead of the HTTP client's own error text. The configured endpoint is no
  longer included in that warning, so URL userinfo, query values, rejected
  header values, and redirect targets cannot reach the diagnostic output. See
  [the users' guide](users-guide.md#femtohttphandler).

### How to update an HTTP handler

- Fix a rejected endpoint or header before upgrading. The error names the
  endpoint or the header field, never its value.
- If a collector answered a 300 or a POST 307/308 in a way the handler treated
  as delivered, make that route answer 2xx directly, or use a 301, 302 or 303.
- A connection waits for at most the smaller of `connect_timeout` and
  `write_timeout`, so `write_timeout` caps connection establishment as well. To
  allow a connection to establish for a required duration, raise
  `write_timeout` to at least that duration, and raise `connect_timeout` too if
  the duration exceeds it.
- If a filter or monitoring rule matches the old transport-error text, or relies
  on the endpoint appearing in that warning, match the fixed classification
  instead. Each handler has exactly one configured endpoint, so identify the
  handler rather than the URL when correlating the warning.

For the full transport contract, see
[the Rust extension guide](rust-extension.md#http-transport-and-tls).

______________________________________________________________________

## What does not need to change

Reading and assigning the five declared fields (`level`, `filename`, `stream`,
`force`, `handlers`) is unchanged. Constructing `BasicConfig` and calling
`basicConfig()` remain unchanged by the slotted-dataclass and `contextvars`
propagation changes. Python-backed handlers registered directly with
`FemtoLogger` must provide contextual-dispatch support; otherwise,
`add_handler()` raises a Python `RuntimeError` with the message
`Python-backed handlers must provide contextual dispatch`, from the Rust
`HandlerError::MissingContextDispatch` variant.

______________________________________________________________________

## Migration checklist

Each item carries the same obligation label as the matching row in
[Table 1](#impact-at-a-glance).

- [ ] **Required:** Handle the fallible Rust logger constructors, and update
      callers that used their results directly as a `FemtoLogger`.
- [ ] **Required if called:** Remove assignments to undeclared `BasicConfig`
      attributes.
- [ ] **Required if called:** Check dynamically built logger names against the
      shape and registry limits.
- [ ] **Required if called:** Rename structured fields that use a built-in
      record name, whether they arrive through `extra=`, `log_context(...)`,
      Rust `push_log_context`, or `RecordMetadata::key_values` passed to
      `log_with_metadata`, and update any `with_record_fields` selection that
      named the old key.
- [ ] **Required for Rust:** Add a wildcard arm to matches on
      `SchemaVersionError`, which is now `#[non_exhaustive]`.
- [ ] **Review if enabled:** Move handler state that relied on a per-logger
      thread to `contextvars` or into the handler itself.
- [ ] **Review if enabled:** Review `StdlibHandlerAdapter` levels, filters, and
      formatters.
- [ ] **Review if enabled:** Move shared enrichment from logger filters to a
      filter on the shared handler.
- [ ] **Review if enabled:** Add explicit synchronization where a file-handler
      call was relied on to exclude other Python threads.
- [ ] **Review if enabled:** Name every logger that must keep emitting,
      because an omitted logger is now disabled.
      `disable_existing_loggers=False` preserves the current state but does
      not re-enable a logger an earlier configuration disabled.
- [ ] **Review if enabled:** Call `flush()` before `close()` on socket and HTTP
      handlers, and accept that a stalled worker drops queued records at the
      single shutdown deadline.
- [ ] **Review if enabled:** Update HTTP handlers for build-time endpoint and
      header validation, the four-redirect limit, and credential stripping on
      redirects.
- [ ] **Review if enabled:** Review Rust struct-literal payload construction
      and callback filters for the new bounded-capture ceilings.
- [ ] **Optional:** Adopt `extra=` and the exception-payload limits.
- [ ] **Optional:** Watch `wait_diagnostics()` and, where the crate is built
      with `tracing-compat`, the `femtologging::internal` wait events to
      diagnose blocked or dropped handler calls.

______________________________________________________________________

## Common errors and fixes

| Error                                                                     | Fix                                                                                                   |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `AttributeError: 'BasicConfig' object has no attribute 'filename_backup'` | Use one of the five declared fields, or keep the value in the caller's own structure.                 |
| `RuntimeError: Python-backed handlers must provide contextual dispatch`   | Give the handler contextual-dispatch support, or register it through `ConfigBuilder` or `dictConfig`. |
| `non-exhaustive patterns` for `SchemaVersionError`                        | Add a wildcard arm; the enum gained nested and limit variants.                                        |

_Table 2: Diagnostics introduced in v0.2.0 and their remedies._
