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
stream = StreamHandlerBuilder.stderr().with_flush_timeout_ms(500).build()

file = FileHandlerBuilder("app.log").with_flush_record_interval(10).build()

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
stream = StreamHandlerBuilder.stderr().with_flush_after_ms(500).build()

file = FileHandlerBuilder("app.log").with_flush_after_records(10).build()

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

The spelling-standardization renames are **non-breaking**. The renamed functions
(`serialize_url_encoded`, `serialize_json` in `http_handler/serialize.rs` and
`serialize_record` in `socket_handler/serialize.rs`) are `pub` within their
respective modules, but the `serialize` modules themselves are private
(`mod serialize`, not `pub mod`), so these symbols are not accessible to
downstream crates. No user-facing method signatures, class names, or parameter
names were altered by this change.

______________________________________________________________________

## New API additions

The following new methods are available on `FemtoLogger` in v0.1.0:

### Convenience logging methods

`debug()`, `info()`, `warning()`, `error()`, `critical()`, and `exception()`
are now available as direct methods on `FemtoLogger`. Each accepts a
pre-formatted `message` string plus optional `exc_info` and `stack_info`
keyword arguments, identical to `log()`. Unlike the stdlib, `*args` /
`**kwargs` lazy formatting is not supported.

```python
logger = get_logger("app")
logger.info("server started")
logger.warning("disk usage above 90%")

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

______________________________________________________________________

## New: standard library (stdlib) logging handler adapter

`StdlibHandlerAdapter` is a new Python class that wraps any `logging.Handler`
subclass for use with femtologging.  It translates femtologging's
`handle_record(dict)` dispatch into the stdlib `handle(LogRecord)` interface.

```python
import logging
from femtologging import FemtoLogger, StdlibHandlerAdapter

adapter = StdlibHandlerAdapter(logging.FileHandler("app.log"))

logger = FemtoLogger("myapp")
logger.add_handler(adapter)
logger.log("INFO", "hello")
```

The adapter honours both thresholds that apply to a wrapped handler: the
femtologging logger's own level, and the stdlib handler's level set with
`logging.Handler.setLevel()`.  Records below the wrapped handler's level are
discarded before reaching it, matching stdlib `Logger.callHandlers()`.

See the [users guide](users-guide.md#using-stdlib-logginghandler-subclasses)
for details and limitations.

______________________________________________________________________

## Default formatter escapes message line separators

### Who should read this

Read this if you rely on multiline message text appearing verbatim in default
formatter output, or if you count physical lines in a log file. The default
formatter now escapes the line separators that Python's `str.splitlines()`
recognizes inside the message field, so text in that field cannot add a line
that looks like another logger's record. The escaped set is LF, CR, VT, FF,
U+001C, U+001D, U+001E, NEL, U+2028, and U+2029; each becomes its
`char::escape_debug` form, such as `\n` or `\u{2028}`. Only the message field
changes: an exception message inside the diagnostics is written unescaped, and
a logger string passed directly to `FemtoFileHandler.handle` is neither
validated nor escaped.

### Impact at a glance

| Obligation         | Area                          | What to do                                                                                          |
| ------------------ | ----------------------------- | --------------------------------------------------------------------------------------------------- |
| Required           | Assertions on default output  | Update expectations that contain raw line breaks in the message field.                              |
| Required if relied | Raw multiline default output  | Configure a percent-style or Python callable formatter when raw message text is required.           |
| Review if enabled  | Size-based rotation           | Allow for longer escaped records when tuning `max_bytes`; escaped bytes count towards the rollover. |
| Review if enabled  | Log parsers and line counting | Treat an escaped message as one physical line; the separator is no longer a record boundary.        |

### 1. Update assertions on default formatter output

The formatter now escapes separators in the message only. Update any expected
string that contained a raw separator in that field.

Before:

```python
assert path.read_text() == "core [INFO] first\nsecond\n"
```

After:

```python
assert path.read_text() == "core [INFO] first\\nsecond\n"
```

The second line still holds a single record terminator. The `\\n` in the
expected value is a backslash followed by `n`, not a line break.

### 2. Restore raw multiline output where required

Only the default formatter escapes. A percent-style `%(message)s` formatter
writes the raw message, and a Python callable formatter receives the raw
message in its record mapping:

```python
from femtologging import FileHandlerBuilder, FormatterBuilder

handler = (
    FileHandlerBuilder("app.log")
    .with_formatter(FormatterBuilder().with_format("%(message)s"))
    .build()
)
```

A callable can keep the logger and level metadata while still writing the raw
message:

```python
def raw_multiline_formatter(record: dict[str, object]) -> str:
    return f"{record['logger']} [{record['level']}] {record['message']}"


handler = FileHandlerBuilder("app.log").with_formatter(raw_multiline_formatter).build()
```

Both forms restore the pre-change line structure, including the ambiguity where
message text can look like another record. Choose them only when a downstream
consumer needs the raw separators and can distinguish the content by other
means.

### What does not need to change

- Percent-style formatters and Python callable formatters receive the raw
  message and are unaffected.
- Structured `handle_record` handlers and the stdlib handler adapter receive
  the raw message and are unaffected.
- Stack and exception diagnostics keep their own multiline structure; only the
  message field is escaped. An exception message is part of the diagnostics, so
  a separator inside it is still written unescaped.
- Logger names passed to `get_logger`/`getLogger` already reject control
  characters and Unicode separators with `ValueError`. A logger string passed
  directly to `FemtoFileHandler.handle` is not validated, so its framing is
  unchanged by this release and still needs a caller-supplied safe name.
- Existing logging calls, handler constructors, and builder methods keep their
  signatures.

### Migration checklist

- [ ] Update assertions that expect raw line breaks in default formatter
  output.
- [ ] Re-count physical lines in log parsers that relied on message separators
  as boundaries.
- [ ] Review `max_bytes` settings for rotating handlers when messages contain
  separators.
- [ ] Configure a formatter where raw multiline message text is required.
- [ ] Re-run traceback and stack tests to confirm diagnostics remain
  multiline.

### Common errors and fixes

This change raises no new exception. An assertion that still expects a raw
separator fails with a value difference such as:

```text
AssertionError: assert 'core [INFO] first\\nsecond\n' == 'core [INFO] first\nsecond\n'
```

The left-hand value is the escaped output and the right-hand value is the old
expectation. Update the expectation to the escaped form, or configure a
formatter that receives the raw message. A parser that reports more records
than it used to is reading the escaped message as one record; treat the `\n`,
`\r`, and `\u{...}` sequences as message text rather than as line boundaries.
