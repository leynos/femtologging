# v0.2.0 migration guide

This document describes breaking changes and structured logging additions in
v0.2.0, with the steps required to update calling code. Existing logging calls
remain valid; the structured logging APIs add optional keyword arguments and
restore scoped-context propagation through named loggers.

______________________________________________________________________

## `BasicConfig` is now a slotted dataclass

`BasicConfig` (exported from `femtologging`) is now declared with
`@dataclasses.dataclass(slots=True)`. Slots remove the per-instance
`__dict__`, so a `BasicConfig` instance can no longer hold attributes beyond
its five declared fields: `level`, `filename`, `stream`, `force`, and
`handlers`. The benefit is that a mistyped or aspirational attribute name now
fails loudly with an `AttributeError` at the point of assignment, instead of
silently creating a dead attribute that the rest of the code never reads.

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
active on the calling thread:

```python
logger = femtologging.get_logger("service")
with femtologging.log_context(request_id=42):
    logger.info("request accepted")
```

The context remains thread-local. In a single-threaded asyncio event loop,
holding it across an `await` can share fields between concurrently in-flight
tasks. Use the per-call `extra` argument for asyncio fields that belong to one
operation. Task-local scoped context backed by `contextvars` remains future
work.

______________________________________________________________________

## Unchanged APIs

Reading and assigning the five declared fields (`level`, `filename`, `stream`,
`force`, `handlers`) is unchanged. `BasicConfig` construction, `basicConfig()`,
and every other public API are unaffected by the slotted-dataclass change.
