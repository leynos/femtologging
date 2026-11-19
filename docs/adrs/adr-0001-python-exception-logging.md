# ADR 0001: Python exception logging support

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
- Walk real exception and frame objects through public Python APIs, collecting
  a bounded structured representation of messages, frames, arguments, notes,
  exception chains, and exception groups.
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

Validation covers the full payload tree. `cause`, `context`, and every
exception-group member are independently versioned, so the call checks each of
them, not only the root. A failure below the root is reported with a path to
the offending payload. The traversal is bounded at 1,000 edges below the root
and 10,000 payload occurrences, matching the recursive frame filter defaults;
those bounds limit validation work and are not parse limits.
`SchemaVersionError` gained nested and limit variants for these failures and is
therefore `#[non_exhaustive]`.

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

Input resolution and traversal have different failure contracts. For a record
that passes the logger threshold, invalid `exc_info` forms still raise
`TypeError` before filters run. Once the input is resolved, missing, malformed,
or raising Python attributes degrade to absent fields or counted omissions. An
unexpected internal traversal failure produces a minimal type-only payload with
`budget_exhausted`; it does not fail the logging call.

Individual argument `repr()` failures are skipped and counted. An exception's
failed `str()` uses `<exception str() failed>`. Notes must be strings; other
items are skipped. Exception-group children that are not exceptions or cannot
be inspected are skipped and counted. Missing frame positions and source lines
remain `None`. Descendants keep empty `args_repr` and `notes`, matching the
formatter contract. Traceback suggestions are not computed.

### Bounded capture and truncation markers

The walker replaces `TracebackException` traversal and applies these defaults
to each capture:

| Component                         | Default ceiling             |
| --------------------------------- | --------------------------: |
| Exception message                 | 4,096 UTF-8 bytes           |
| Argument representations          | 32 items, 512 bytes each    |
| Notes                             | 32 items, 512 bytes each    |
| Frames                            | 64 per traceback or stack   |
| Filename / function / source line | 512 / 256 / 512 UTF-8 bytes |
| Cause or context links            | 32 levels                   |
| Exception-group nesting / width   | 10 levels / 15 children     |
| Exception nodes / aggregate text  | 256 nodes / 256 KiB         |

The ASCII `...` suffix fits inside each text limit. A fixed-size optional
`truncation` marker records shortened messages, argument representations, and
notes; omitted arguments, notes, frames, links, or group members; failed
elements; and aggregate-budget exhaustion. Identifiers that are shortened
already carry a visible `...` suffix. The schema remains version 1 because the
marker is optional and defaults to absent. Absent markers leave existing
formatter output unchanged.

Built-in strings, bytes, and bytearrays are sliced or escaped from bounded
prefixes before capture retains them. `KeyError` is handled without invoking
its built-in `__str__`, because that implementation calls `repr` on the whole
argument; the message is rebuilt from the same bounded argument rendering
instead. General user-defined `str()` and `repr()` calls still execute on the
caller thread. Their *number* is bounded by the item and traversal limits, and
their failures are contained, but a single user conversion can take arbitrary
time or allocate internally before it returns.

Capture spends the aggregate budget in this order: type, message, frames,
arguments, notes, cause, context, and group children. It keeps the innermost
frames in a fixed-capacity ring, limits chain and group depth, caps group width
and total nodes, and detects identity cycles. These limits bound retained data
and the number of user conversions; they cannot pre-empt a conversion already
running in Python.

## Known risks and limitations

### Local variable capture is disabled (as of 0.1.0)

The `StackFrame` schema retains a `locals` field, but the bounded walker always
records it as `None`. It builds frames directly from public frame and
code-position APIs instead of `TracebackException` summaries.

This is intentional. Enabling locals capture carries three concerns that have
not yet been addressed:

1. **Security — accidental credential leakage.** Local variables
   frequently contain sensitive values such as API keys, database passwords,
   authentication tokens, and personally identifiable information. Calling
   `repr()` on these and serializing them into log output risks exposing
   credentials to log aggregation systems, files, or third-party observability
   pipelines.

2. **Payload size.** Each frame's local variables are serialized as
   string representations. In frames with large data structures, NumPy arrays,
   or Django querysets, the `repr()` output can be arbitrarily large, inflating
   log record sizes and increasing memory pressure on the crossbeam channel
   queues.

3. **Performance overhead.** `repr()` is called on every local
   variable in every frame of the traceback. For deep call stacks or frames
   with many locals, this adds non-trivial latency to the caller thread — the
   exact path the library is designed to keep fast.

Enabling locals capture would require a separate opt-in and redaction design.
No timeline is committed for this work.

## Consequences

- The fast path remains unchanged when exception and stack data are absent.
- Memory use increases for records with structured exception or stack payloads.
- Worker threads remain GIL-free because only Rust-owned data is queued.
- Formatter and handler interfaces must account for the schema version and
  optional structured payload fields.
