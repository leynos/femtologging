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

## Handler `close()` is bounded end to end

`FemtoSocketHandler.close()` and `FemtoHTTPHandler.close()` now run on one
deadline, taken from the handler's write timeout, that covers the whole
shutdown: signalling the worker, queueing the acknowledgement request, waiting
for the acknowledgement, and joining the worker. Previously the acknowledgement
wait and the join each had their own or no bound, so a worker stuck in a
blocking network call could hold `close()` for far longer than the timeout
suggested. `Drop` calls `close()`, so the same bound applies to a handler that
is simply dropped.

A worker that does not acknowledge within the budget is abandoned rather than
joined. It finishes the network call it was in, sends what is still queued up
to the deadline, discards the rest, and exits on its own with nothing left to
join it. This changes three observable behaviours:

- **`close()` returns on time.** Code that relied on `close()` blocking until
  every queued record had been sent must call `flush()` first, and must accept
  that a flush is best-effort under the same timeout.
- **Records can be dropped at the deadline.** Whatever is still queued when the
  drain deadline passes is discarded rather than sent, and one rate-limited
  warning reports how many records went that way. A collector that cannot keep
  up with the shutdown budget now loses records instead of extending shutdown.
- **A partially written frame stays partially written.** A frame that runs out
  of budget mid-write reports how many of its bytes were sent. The connection
  is closed rather than retried, so the collector may observe a truncated frame.

The bound is on what the worker *starts*: no standard-library API cancels a
blocking `connect`, `write`, or TLS handshake, so a close can take up to the
budget plus one network call already in flight when the deadline fell. Nothing
new starts after the deadline.

No API changed. The contract is unchanged for callers that already treat
`close()` as best-effort, and `flush()` keeps its existing boolean return.

______________________________________________________________________

## `StdlibHandlerAdapter` honours the wrapped handler level

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

## `StdlibHandlerAdapter` propagates `contextvars`

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

______________________________________________________________________

## Bounded exception-payload filtering

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

______________________________________________________________________

## Production wheels omit the timed-rotation test hooks

The persistent `[tool.maturin]` feature set is now `["python"]` rather than
`["test-util"]`. Wheels and the source distribution therefore no longer export
`set_timed_rotation_test_times_for_test` or
`clear_timed_rotation_test_times_for_test`, and `femtologging._has_test_util` is
`False`. Their underscore-prefixed Python wrappers,
`_set_timed_rotation_test_times_for_test` and
`_clear_timed_rotation_test_times_for_test`, are retained as compatibility
shims: the setter raises `RuntimeError` because the feature is absent, and the
clearer does nothing.

These helpers were never supported runtime API. Code that relied on them must
build the extension with the `test-util` feature, either by setting
`UV_CONFIG_FILE=.github/uv-test-util.toml` for a source install or by passing
`--features python,test-util` to a development build. The rotating
fresh-failure helpers `_force_rotating_fresh_failure_for_test` and
`_clear_rotating_fresh_failure_for_test` remain exposed in every build, but
they are test-only as well and carry no support guarantee.

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
  handlers, filters, and enabled state.
- Move diagnostic calls to a configured logger, or add the emitting logger to
  the configuration, instead of relying on propagation from an omitted logger.
- Do not rely on a configuration rollback to retract records already queued or
  to undo a file or socket opened during handler construction.

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
