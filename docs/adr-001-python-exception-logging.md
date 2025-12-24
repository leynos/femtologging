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
without re-parsing text.

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
re-parsing text. The initial schema should be versioned to allow evolution
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

## Consequences

- The fast path remains unchanged when exception and stack data are absent.
- Memory use increases for records with structured exception or stack payloads.
- Worker threads remain GIL-free because only Rust-owned data is queued.
- Formatter and handler interfaces must account for the schema version and
  optional structured payload fields.
