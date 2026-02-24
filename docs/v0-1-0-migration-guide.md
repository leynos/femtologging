# v0.1.0 migration guide

This document describes breaking changes introduced in v0.1.0 and the steps
required to update calling code.

______________________________________________________________________

## Builder method renames

The flush-related builder methods have been renamed to follow a consistent
`with_flush_after_*` pattern. The semantics are unchanged: file handlers flush
after a record count; stream handlers flush after a time interval.

Table: Builder method renames

| Builder                      | Old method                   | New method                 |
| ---------------------------- | ---------------------------- | -------------------------- |
| `StreamHandlerBuilder`       | `with_flush_timeout_ms`      | `with_flush_after_ms`      |
| `FileHandlerBuilder`         | `with_flush_record_interval` | `with_flush_after_records` |
| `RotatingFileHandlerBuilder` | `with_flush_record_interval` | `with_flush_after_records` |

### Before

```python
stream = (
    StreamHandlerBuilder.stderr()
    .with_flush_timeout_ms(500)
    .build()
)

file = (
    FileHandlerBuilder("app.log")
    .with_flush_record_interval(10)
    .build()
)

rotating = (
    RotatingFileHandlerBuilder("app.log")
    .with_flush_record_interval(10)
    .with_max_bytes(10_000_000)
    .with_backup_count(3)
    .build()
)
```

### After

```python
stream = (
    StreamHandlerBuilder.stderr()
    .with_flush_after_ms(500)
    .build()
)

file = (
    FileHandlerBuilder("app.log")
    .with_flush_after_records(10)
    .build()
)

rotating = (
    RotatingFileHandlerBuilder("app.log")
    .with_flush_after_records(10)
    .with_max_bytes(10_000_000)
    .with_backup_count(3)
    .build()
)
```

______________________________________________________________________

## `as_dict()` key changes

The dictionary keys returned by `as_dict()` on each builder have changed to
match the new method names.

Table: Dictionary key renames

| Builder                                             | Old key                 | New key               |
| --------------------------------------------------- | ----------------------- | --------------------- |
| `StreamHandlerBuilder`                              | `flush_timeout_ms`      | `flush_after_ms`      |
| `FileHandlerBuilder` / `RotatingFileHandlerBuilder` | `flush_record_interval` | `flush_after_records` |

Code that inspects or asserts on builder dictionaries must update its key
lookups accordingly.

______________________________________________________________________

## Error message changes

Validation error messages now reference the new parameter names:

Table: Error message changes

| Old message                                       | New message                                     |
| ------------------------------------------------- | ----------------------------------------------- |
| `flush_timeout_ms must be greater than zero`      | `flush_after_ms must be greater than zero`      |
| `flush_record_interval must be greater than zero` | `flush_after_records must be greater than zero` |

Code that matches on these strings (e.g. in `pytest.raises(match=…)`) must be
updated.

______________________________________________________________________

## Unchanged APIs

The following APIs are **not** affected by this change:

- `FemtoFileHandler(path, capacity=…, flush_interval=…, policy=…)` —
  the direct constructor parameter `flush_interval` is unchanged.
- `HandlerOptions(capacity=…, flush_interval=…, policy=…)` — the
  options struct parameter is unchanged.
- `handler.flush()` and `handler.close()` — instance methods are
  unchanged.
- All `with_capacity()`, `with_overflow_policy()`, `with_formatter()`,
  `with_max_bytes()`, and `with_backup_count()` builder methods are unchanged.

______________________________________________________________________

## Search-and-replace recipe

The following commands apply the required renames mechanically. They assume GNU
sed; on macOS replace `sed -i` with `sed -i ''`.

```bash
# Method calls
find . -name '*.py' -exec sed -i \
    's/\.with_flush_timeout_ms(/.with_flush_after_ms(/g' {} +
find . -name '*.py' -exec sed -i \
    's/\.with_flush_record_interval(/.with_flush_after_records(/g' {} +

# Dictionary keys
find . -name '*.py' -exec sed -i \
    's/"flush_timeout_ms"/"flush_after_ms"/g' {} +
find . -name '*.py' -exec sed -i \
    's/"flush_record_interval"/"flush_after_records"/g' {} +

# Error message match strings
find . -name '*.py' -exec sed -i \
    's/flush_timeout_ms must be/flush_after_ms must be/g' {} +
find . -name '*.py' -exec sed -i \
    's/flush_record_interval must be/flush_after_records must be/g' {} +
```

______________________________________________________________________

## Rust API changes

For consumers of the Rust crate directly, the public method signatures have
changed:

Table: Rust API renames

| Struct                       | Old method                   | New method                 |
| ---------------------------- | ---------------------------- | -------------------------- |
| `StreamHandlerBuilder`       | `with_flush_timeout_ms`      | `with_flush_after_ms`      |
| `FileHandlerBuilder`         | `with_flush_record_interval` | `with_flush_after_records` |
| `RotatingFileHandlerBuilder` | `with_flush_record_interval` | `with_flush_after_records` |

Internal field names (`flush_after_ms`, `flush_after_records`) and setter
methods (`set_flush_after_records`) have also changed but are `pub(crate)` and
not part of the public API.

______________________________________________________________________

## Spelling standardization (-ise to -ize)

All identifiers, documentation, and internal names have been standardized to
use Oxford English Dictionary (-ize) spelling. This follows the project style
guide (`en-GB-oxendict`).

### Rust module renames

Consumers of the Rust crate who use `mod` paths or `include!` macros
referencing internal module files should note the following renames:

Table: Rust module file renames

| Parent module    | Old file       | New file       |
| ---------------- | -------------- | -------------- |
| `http_handler`   | `serialise.rs` | `serialize.rs` |
| `socket_handler` | `serialise.rs` | `serialize.rs` |

These modules are private (`mod serialize`, not `pub mod`) and not re-exported,
so this change does not affect Python consumers or users of the public Rust API.

### Documentation and docstring changes

All doc comments and user-facing strings now use `-ize` / `-ization` forms:

- `serialise` / `serialisation` → `serialize` / `serialization`
- `normalise` / `normalisation` → `normalize` / `normalization`
- `initialise` / `initialisation` → `initialize` / `initialization`
- `finalise` → `finalize`
- `customise` → `customize`
- `maximise` → `maximize`
- `recognise` → `recognize`

### Impact

The spelling-standardization renames are **non-breaking**. The renamed
functions (`serialize_url_encoded`, `serialize_json` in
`http_handler/serialize.rs` and `serialize_record` in
`socket_handler/serialize.rs`) are `pub` within their respective modules, but
the `serialize` modules themselves are private (`mod serialize`, not
`pub mod`), so these symbols are not accessible to downstream crates. No
user-facing method signatures, class names, or parameter names were altered by
this change.

______________________________________________________________________

## New API additions

The following new methods are available on `FemtoLogger` in v0.1.0:

### Convenience logging methods

`debug()`, `info()`, `warning()`, `error()`, `critical()`, and `exception()`
are now available as direct methods on `FemtoLogger`. Each accepts the same
`exc_info` and `stack_info` keyword arguments as `log()`.

```python
logger = get_logger("app")
logger.info("server started")
logger.warning("disk usage above 90%%")

try:
    risky_call()
except Exception:
    logger.exception("risky_call failed")
```

`exception()` behaves like `error()` but defaults `exc_info` to `True`,
automatically capturing the active exception.

### `isEnabledFor`

```python
if logger.isEnabledFor("DEBUG"):
    logger.debug(f"expensive computation: {compute()}")
```

### `getLogger` alias

`getLogger` is available as a module-level alias for `get_logger`, matching the
standard library naming convention:

```python
from femtologging import getLogger

logger = getLogger("app.module")
```
