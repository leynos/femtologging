# v0.2.0 migration guide

This document describes breaking changes and structured logging additions in
v0.2.0, with the steps required to update calling code. Existing logging calls
remain valid; the structured logging APIs add optional keyword arguments and
restore scoped-context propagation through named loggers.

______________________________________________________________________

## `BasicConfig` is now a slotted dataclass

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

## Inline structured fields

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

______________________________________________________________________

## Scoped context through named loggers

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

## Scoped logging context follows Python tasks

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

## Bounded logger names and shared dispatch

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

## `StdlibHandlerAdapter` propagates `contextvars`

When a record is sent through `StdlibHandlerAdapter`, femtologging captures the
producer thread's `contextvars` context before queueing the record. On the
logger's shared dispatch-shard worker, `Context.run` calls the wrapped stdlib
handler's complete `handle()` method inside that context, including its
filters, formatters, and `emit()` method. Dispatch remains asynchronous.
Existing filters and formatters that read a `contextvars.ContextVar` therefore
observe the value set by the thread that emitted the record.

No code changes are required for this behaviour. Only `contextvars` are
propagated; other thread-local state, including `threading.local`, belongs to
the shared shard worker and may be observed by handlers serving different
loggers. Update filters that depend on producer state to use `contextvars`.

______________________________________________________________________

## Handler filters and formatter builders

Handler builders now accept filter IDs with `with_filters([...])`. Register
those IDs with `ConfigBuilder`, or declare them in the top-level `filters`
section when using `dictConfig`; handler `filters` lists can then refer to
them. The filter chain runs on the producer thread before enqueueing and also
runs for records propagated from child loggers to an ancestor handler. This
makes a shared handler a single attach point for cross-cutting enrichment.
Logger filters remain local to the logger that received the call. For the
configured attachment order and per-handler snapshot semantics, see the
accepted handler-filter addendum in
[ADR-003](adr-003-python-stdlib-filter-parity.md#accepted-addendum-2026-10-14-handler-filters).

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

______________________________________________________________________

## Deadline-aware Rust handler flushing

`FemtoLogger::flush_handlers()` now applies one two-second deadline to marker
insertion, the non-empty handler snapshot, worker acknowledgement and every
handler flush. It returns `false` when any part of that barrier is unconfirmed.
Built-in handlers keep their shorter configured timeouts.

Custom Rust handlers that implement only synchronous `flush()` now fail closed:
the barrier does not call that method and returns `false`. Override
`flush_until(deadline)` to opt in. The implementation must use bounded waits
against the supplied absolute deadline and report `false` when completion is
not confirmed. The logger cannot interrupt arbitrary synchronous handler code
once it starts; calling an unbounded `flush()` from `flush_until()` can still
occupy the shared worker after the caller returns. In normal builds, logger
drop waits only for that logger's in-flight records, up to the bounded drain
timeout; it does not join the process-wide worker. Loom models join their
logger-owned worker during cleanup.

Python-facing calls release the GIL while waiting for the worker. CPython must
reacquire the interpreter before returning to Python, so interpreter contention
can delay the Python return beyond the Rust barrier deadline; a late handoff
changes the result to `false`.

See the [user guide](users-guide.md#flushing-logger-handlers), the
[developer guide](developers-guide.md#logger-handler-flush-barrier), and the
[Rust handler port guide](formatters-and-handlers-rust-port.md#femtohandler-trait-and-implementations)
for the public contract and implementation details.

______________________________________________________________________

## Unchanged APIs

Reading and assigning the five declared fields (`level`, `filename`, `stream`,
`force`, `handlers`) is unchanged. Constructing `BasicConfig` and calling
`basicConfig()` remain unchanged by the slotted-dataclass and `contextvars`
propagation changes. Python-backed handlers registered directly with
`FemtoLogger` must provide contextual-dispatch support; otherwise,
`add_handler()` raises a Python `RuntimeError` with the message
`Python-backed handlers must provide contextual dispatch`, from the Rust
`HandlerError::MissingContextDispatch` variant.
