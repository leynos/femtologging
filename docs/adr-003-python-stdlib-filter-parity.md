# Architectural decision record (ADR) 003: Python standard library filter parity

## Status

Accepted – decision recorded on 2026-02-25 to add Python standard library
(`logging`) filter parity in a phased rollout.

## Date

2026-02-25.

## Context and problem statement

`femtologging` currently supports two Rust-native filter builders:
`LevelFilterBuilder` and `NameFilterBuilder`. This model works for basic
severity and namespace gating, but it does not support Python callback filters
such as `logging.Filter` subclasses or plain callables.

This limitation blocks direct interoperability with middleware patterns that
store per-request values in `contextvars` and inject those values into log
records via a filter. A representative example is the Falcon correlation ID
middleware design, which expects a contextual `logging.Filter` to set
`correlation_id` and `user_id` on each record before formatting.[^1]

An architectural decision is needed to define how parity with Python standard
library (stdlib) filter behaviour should be implemented without violating
femtologging's core constraints around asynchronous dispatch, thread safety,
and predictable performance.

## Decision drivers

- Improve compatibility with existing Python logging ecosystems.
- Enable middleware-driven contextual enrichment (for example, request
  correlation identifiers and authenticated user identifiers).
- Preserve the non-blocking producer-consumer architecture.
- Keep worker threads free of Python object lifetimes and sustained Global
  Interpreter Lock (GIL) coupling.
- Maintain explicit and testable configuration semantics across `ConfigBuilder`,
  `dictConfig`, and `fileConfig`.

## Requirements

### Functional requirements

- Allow logger and root filters to be defined as Python stdlib-compatible
  callback filters (`logging.Filter` instances or callables).
- Preserve existing Rust-native filters (`LevelFilterBuilder`,
  `NameFilterBuilder`) for low-overhead filtering.
- Allow Python filters to enrich records with contextual fields that formatters
  and structured handlers can read.
- Support `dictConfig` filter factories (`"()"`) in addition to existing
  declarative `level` and `name` forms.

### Technical requirements

- Execute Python filter callbacks on the producer thread before queueing.
- Avoid retaining Python objects across queue boundaries.
- Keep worker-thread dispatch independent of Python callbacks.
- Provide deterministic error handling for callback failures.
- Preserve current handler-level filter constraints unless explicitly added in a
  separate decision.

## Options considered

### Option A: retain Rust-only filter model

Keep filters restricted to `LevelFilterBuilder` and `NameFilterBuilder`, and
recommend formatter-level or application-level context injection as a
workaround.

### Option B: add Python callback filter parity on the producer path

Add a Python-backed filter type that can execute `filter(record)` or callable
forms during producer-side evaluation, and persist allowed record enrichments
into Rust-owned record metadata before queueing.

### Option C: execute Python callback filters in worker threads

Queue raw records first, then run Python callback filters inside handler worker
threads before formatting or emission.

| Criterion                                  | Option A | Option B | Option C |
| ------------------------------------------ | -------- | -------- | -------- |
| Python stdlib compatibility                | Low      | High     | High     |
| Worker-thread isolation from Python        | High     | High     | Low      |
| Contextvars interoperability               | Low      | High     | Medium   |
| Architecture fit with existing async model | Medium   | High     | Low      |
| Runtime implementation risk                | Low      | Medium   | High     |

_Table 1: Trade-offs for Python stdlib filter parity strategies._

## Decision outcome / proposed direction

Adopt Option B.

`femtologging` will add Python callback filter parity by evaluating Python
filters on the producer thread and persisting accepted enrichments into
Rust-owned record data before asynchronous dispatch.

The chosen direction includes:

- A Python-backed filter adapter that accepts either:
  - objects exposing `filter(record)`, or
  - callables that take one record argument and return truthy/falsy values.
- A mutable record view for callback execution that mirrors stdlib expectations
  for record attribute enrichment.
- Serialization of accepted enrichment fields into record metadata so Python
  formatters and handlers can read them later without Python object sharing
  across threads.
- Explicit enrichment persistence constraints:
  - keys must be strings and must not collide with stdlib `LogRecord`
    attributes or femtologging-reserved metadata keys;
  - values may be `str`, `int`, `float`, `bool`, or `None` only, with
    non-string scalar values converted to strings before persistence;
  - enrichment is bounded to 64 keys per record, 64 UTF-8 bytes per key,
    1,024 UTF-8 bytes per value, and 16 KiB total serialized enrichment payload
    per record.
- `dictConfig` support for stdlib-style filter factories (`"()"`) alongside
  existing `level` and `name` forms.
- `dictConfig` conflict handling semantics:
  - entries using `"()"` are factory-mode entries and must not include `level`
    or `name`;
  - entries without `"()"` must include exactly one of `level` or `name`;
  - mixed or ambiguous forms are rejected with `ValueError` (no precedence
    ordering is applied).

## Goals and non-goals

### Goals

- Enable integration with contextvar-driven middleware filter patterns.
- Preserve existing Rust filter performance paths for users that do not need
  Python callback filters.
- Provide explicit parity boundaries in documentation and tests.

### Non-goals

- Handler-level stdlib filter parity in this decision.
- Full stdlib `LogRecord` object parity across every field and internal
  implementation detail.
- Cross-thread propagation of arbitrary Python objects embedded in records.

## Migration plan

### Phase 1: producer-path callback filter support

Introduce Python callback filter adapters and producer-side evaluation with
record enrichment persistence to Rust-owned metadata.

### Phase 2: configuration parity expansion

Extend `dictConfig` filter parsing to support stdlib factory forms (`"()"`) and
document expected compatibility behaviour, including the mixed-form rejection
rules defined in this ADR.

### Phase 3: conformance and hardening

Add integration tests for contextvar-based enrichment flows, concurrency
safety, error semantics, and performance regression coverage.

## Known risks and limitations

- Producer-side callback execution increases hot-path work for configurations
  using Python filters.
- Partial stdlib parity may still leave behavioural differences for rare
  `logging` edge cases.
- Enrichment bound enforcement can reject callback-produced fields when limits
  are exceeded.

## Architectural rationale

This direction preserves femtologging's core architecture by keeping Python
interaction on the producer side and avoiding Python object retention in worker
threads. It also closes a practical interoperability gap with middleware that
depends on stdlib filter semantics for contextual logging.

[^1]: <https://raw.githubusercontent.com/leynos/falcon-correlate/refs/heads/implement-contextual-log-filter-nlw4m4/docs/falcon-correlation-id-middleware-design.md>
