# ADR 001: Python exception logging support

## Status

- Proposed (2025-12-24)

## Context

This Architecture Decision Record (ADR) captures how femtologging should add
support for Python `exc_info` and `stack_info` logging parameters.

The current Python API only accepts a message string, and the Rust
`FemtoLogRecord` only carries that message plus basic metadata. Exception and
stack data are therefore unavailable to formatters and handlers. The library
uses asynchronous Rust worker threads, and the design aims to keep those worker
threads free of the Global Interpreter Lock (GIL) and Python object lifetimes.

PyO3 (Python to Rust bindings) provides the foreign function interface between
Python and Rust. The exception and stack data must be captured in the caller
thread because they depend on the active Python exception and stack state.

## Decision drivers

- Preserve the fast path when `exc_info` and `stack_info` are not requested.
- Avoid retaining Python objects across threads or extending exception
  lifetimes.
- Keep worker threads GIL-free whenever possible.
- Maintain compatibility with standard logging semantics for exception and
  stack formatting.
- Integrate with existing Rust formatters and the Python formatter adapter.

## Considered options

### Option A: Eager Python serialization to Rust-owned strings

Capture exception and stack data in the caller thread, serialize to strings,
store the results in the `FemtoLogRecord`, and let formatters append the text
when present.

Pros:

- Captures the correct Python exception and stack context at the call site.
- Keeps worker threads free of Python object lifetimes and GIL usage.
- Fits the existing record and formatter model with minimal changes.
- Provides deterministic payload sizes and straightforward testing.

Cons:

- Adds overhead on the call path when enabled.
- Duplicates work if multiple handlers reformat the same record.
- Limits access to structured traceback data unless extra fields are added.

### Option A.2: Semantic stack trace serialization

Capture exception and stack data in the caller thread, serialize to a semantic
representation (for example, frames, locals, exception type, and message), and
store the structured payload in the `FemtoLogRecord`. Formatters can then
render either a human-readable string or structured output (such as JSON)
without reparsing text.

Pros:

- Preserves the full stack trace details for multiple formatter styles.
- Avoids repeated parsing or reformatting across handlers.
- Keeps worker threads free of Python object lifetimes and GIL usage.

Cons:

- Increases record size and serialization overhead.
- Requires a stable schema for exceptions and stack frames.
- Adds complexity to formatter and handler interfaces.

### Option B: Deferred formatting with PyO3 exceptions

Capture `exc_info` as a Python object, store it in the queued record, and
format the traceback in the handler thread by acquiring the GIL.

Pros:

- Defers formatting work until the record is processed.
- Enables richer formatting by keeping structured exception data.

Cons:

- Requires GIL acquisition on worker threads, which undermines the Rust-only
  worker design.
- Retains Python objects and frames across threads, risking memory growth and
  longer exception lifetimes.
- Increases complexity in `Send` and `Sync` guarantees for queued records.

### Option C: Rust-only backtrace capture for `stack_info`

Capture a Rust backtrace instead of Python frames and attach it to the record.

Pros:

- Avoids Python overhead and object retention.
- Keeps stack capture fully in Rust.

Cons:

- Does not reflect Python call stacks, which are the expected semantics for
  `stack_info`.
- Does not address `exc_info`, which is tied to Python exceptions.

## Decision

Adopt Option A.2.

The structured payload is preferred because it keeps the worker threads
GIL-free while enabling both human-readable and structured renderings without
reparsing text. The initial schema should be versioned to allow evolution
without breaking formatters.

The implementation should serialize exception and stack data in the caller
thread, store it in the record as Rust-owned strings, and let formatters attach
the extra text. This keeps the asynchronous logging model intact while
providing output aligned with standard logging behaviour.

## Implementation sketch

- Extend the Python `FemtoLogger.log` signature to accept keyword-only
  `exc_info` and `stack_info` parameters.
- Accept `exc_info` values matching standard logging behaviour: `True` to use
  `sys.exc_info()`, an exception instance, or a three-item exception tuple.
- Use Python's `traceback` helpers to collect a structured representation of
  the exception and stack, preserving frames, code context, and exception
  chaining where available.
- Add optional structured fields to `FemtoLogRecord` for exception and stack
  payloads, plus a `schema_version` marker to allow evolution.
- Extend the default Rust formatter and the Python formatter adapter to render
  both a concise human-readable string and structured output (for example, JSON
  fields) from the stored payload.
- Keep the existing Python handler `handle(logger, level, message)` signature,
  but add an optional `handle_record(record: Mapping)` hook for handlers that
  need direct access to the structured payload.
- Detect the presence of `handle_record` **once at registration time** and cache
  the result. This avoids per-record attribute lookups and keeps the hot path
  fast. Handlers must be fully configured before calling `add_handler()`;
  adding or removing `handle_record` after registration results in undefined
  behaviour.

## Schema Versioning

### Version Policy

The exception and stack trace payload schemas are versioned to allow evolution
without breaking consumers. The `schema_version` field is included in every
payload.

- **Backward compatibility**: Newer code can always read older payloads.
  Optional fields use serde defaults when absent.
- **Forward compatibility**: Not guaranteed. Code should validate the schema
  version before processing payloads from external sources.

### Validation

Consumers deserializing payloads from network or storage should call
`validate_version()` to ensure compatibility:

```rust
let payload: ExceptionPayload = serde_json::from_str(&json)?;
payload.validate_version()?;
```

If the version is unsupported, the error message includes both the observed and
supported versions for diagnostic purposes.

### Breaking Changes

The schema version must be incremented when:

- Adding required fields
- Removing fields
- Changing field types or semantics
- Renaming fields

Adding optional fields with `#[serde(default)]` does not require a version bump.

## Graceful degradation rules

The capture logic intentionally degrades gracefully when Python objects are
malformed, missing attributes, or contain values that cannot be extracted to
the expected Rust types. This design ensures the logging system remains
operational even when tracebacks have unusual shapes.

### Required vs optional extraction

Fields fall into two categories:

**Required fields** cause extraction to fail if missing or malformed. The
entire frame or exception capture aborts with an error:

- Stack frame: `filename`, `lineno`, `name` (function name)
- Exception: `exc_type`, `__name__` (for type name)

**Optional fields** degrade silently to `None` or an empty collection when
missing, `None`, or the wrong type:

- Stack frame: `end_lineno`, `colno`, `end_colno`, `line` (source line),
  `locals`
- Exception: `module`, `args`, `__notes__`, `__cause__`, `__context__`,
  `exceptions` (for `ExceptionGroup`)

### The `get_optional_attr` helper

The `get_optional_attr<T>` function encapsulates the degradation logic for
optional attributes. It returns `None` in all the following cases:

1. The attribute does not exist on the Python object.
2. The attribute value is Python `None`.
3. The attribute exists but cannot be extracted to type `T`.

This single helper eliminates repetitive error handling and guarantees
consistent behaviour across all optional field extractions.

### Partial extraction for collections

When extracting collection fields, individual entries that fail are skipped
rather than aborting the entire collection:

- **`locals` dictionary:** Entries with non-string keys or values whose
  `repr()` fails are skipped. Valid entries are preserved. An empty result
  becomes `None`.
- **`args_repr` list:** If `.args` is missing, `None`, or not a tuple, the
  result is an empty vector. Individual elements that fail `repr()` extraction
  are skipped.
- **`notes` list:** If `__notes__` is missing or `None`, the result is an empty
  vector. Individual elements that are not strings are skipped.
- **`exceptions` list (ExceptionGroup):** Individual nested exceptions that
  fail to convert are skipped from the result vector.

### Design rationale

This approach was chosen for several reasons:

1. **Python version compatibility:** Enhanced traceback fields (`end_lineno`,
   `colno`, `end_colno`) were added in Python 3.11. Graceful degradation allows
   the same code to work on older Python versions without version checks.

2. **Partial data over total failure:** When capturing diagnostic information,
   having some data is better than none. A traceback with missing column
   numbers is still useful; a completely failed capture is not.

3. **Resilience to unusual objects:** Exception subclasses may override
   attributes in unexpected ways. Defensive extraction prevents edge cases from
   crashing the logger.

4. **Logging must not fail:** The logging system is the last line of defence
   for diagnostics. If capturing exception details fails, the original error
   context is lost entirely. Graceful degradation ensures the log record is
   still emitted with whatever data was recoverable.

## Consequences

- The fast path remains unchanged when exception and stack data are absent.
- Memory use increases for records with structured exception or stack payloads.
- Worker threads remain GIL-free because only Rust-owned data is queued.
- Formatter and handler interfaces must account for the schema version and
  optional structured payload fields.
