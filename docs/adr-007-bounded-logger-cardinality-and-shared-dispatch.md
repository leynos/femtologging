# Architectural decision record (ADR) 007: bounded logger cardinality and shared dispatch

## Status

Accepted — 2026-10-16: bound logger names, cap the registry, and replace the
one-worker-thread-per-logger model with a fixed pool of shared dispatch shards.

## Date

2026-10-16

## Context and problem statement

Three properties of the logger combined into one resource-amplification defect,
reported as issue #432 and consolidating the findings `892a59c0` (unbounded
creation through the convenience `name=` route), `a25c11d3` (thread-per-logger
denial of service), `b721c5d5` (unbounded registry retention) and `193b1941`
(control-character injection through logger names).

**The registry retained every name it was ever asked for.** The process-global
manager held a `HashMap<String, Py<FemtoLogger>>` and inserted on every
`get_logger(name)` miss, evicting nothing until `reset_manager()` or process
exit. Name validation rejected only empty, leading-dot, trailing-dot and
consecutive-dot names, so it bounded the *shape* of a name and not its size or
its number.

**Every logger owned an operating-system thread.** Construction built a
1,024-slot queue and unconditionally called `thread::spawn()`, so registry
cardinality *was* thread cardinality. Ten thousand loggers meant ten thousand
threads, each with its own stack.

**The names are frequently not the application's to choose.** Logger names are
derived from tenant, request, user, route and plug-in identifiers in ordinary
code, and the Rust `log` and `tracing` bridges derive them from a crate's
`target`. A library that logs per-target inherits a field whose cardinality its
caller controls. `log::debug!(target: request_id, ...)` compiles, reads
naturally, and leaks.

The three properties multiply rather than add. An unbounded key space is a
memory leak; the same key space driving a thread per entry is a thread
exhaustion for any caller whose input is attacker-influenced. And
`thread::spawn()` failing under that pressure panics, so the failure mode of
being attacked is a crash rather than degraded logging.

This was not theoretical for the convenience route:
`femtologging.info(…, name=…)` reached the same registry, so a call whose
`name` argument looked like a parameter accepted unbounded distinct values for
it.

## Decision

Bound every axis along which the registry and the dispatch machinery can grow,
and refuse rather than degrade, so that a process using the logging system with
attacker-influenced names consumes a bounded amount of memory and a bounded
number of threads.

### Bounded names

A logger name is validated before it reaches the registry, on every creation
path, against three rules in cheapest-first order:

| Rule                        | Bound                                                      | Rejection    |
| --------------------------- | ---------------------------------------------------------- | ------------ |
| UTF-8 byte length           | 255 bytes                                                  | `ValueError` |
| Dot-separated segment count | 32 segments                                                | `ValueError` |
| Control characters          | none (all of `char::is_control()`, plus `U+2028`/`U+2029`) | `ValueError` |

The byte check runs first so a hostile megabyte-long name is refused without
ever being scanned.

The control-character ban is not cosmetic. A logger name is rendered into every
formatted record, so a name containing a newline lets a caller forge a record
boundary in line-oriented output — the injection a log consumer is least able
to detect. The diagnostic for a rejected name quotes a bounded, control-escaped
prefix for the same reason: a rejection message must not reintroduce the
injection the rule exists to stop. The `log` bridge's refusal warning applies
the same quoting to the refused target, which is the more hostile of the two: a
target need not have passed validation to reach it.

The ban is stated in terms of line breaks a consumer recognizes, not in terms
of ASCII. Python's `str.splitlines()` breaks on `U+0085`, `U+2028` and `U+2029`
as well as on `\n`, so a rule that refused only the ASCII range would leave the
forgery available through a character the consumer still treats as a boundary.

The segment bound encodes an intent worth stating. Logger names describe
*component paths*, which are shallow. A 32-segment name is not a deeper
hierarchy; it is a request-derived string wearing a hierarchy's punctuation.

### Bounded registry

The registry holds at most 10,000 named loggers (`MAX_LOGGERS`) and **never
evicts**. Once the cap is reached, a request for a name that does not yet exist
is refused with `RuntimeError`.

Eviction was considered and rejected, and the reason is the whole design. A
logger is not a cache entry: it owns its handler set, its level, its
propagation flag and its drop counters, all of which are state a caller may
still hold a reference to and still be using. Displacing one would silently
redirect its records — records would go somewhere the caller did not configure,
with no error and no way to notice. A bound that corrupts behaviour when it
engages is worse than no bound.

Refusal also makes the failure legible. A `RuntimeError` naming the cap and
suggesting the fix tells a caller that its naming scheme, not the logging
system, is the problem, and it does so on the request that crossed the line
rather than later, when memory runs out.

`"root"` is exempt from the count and is created on demand. It is the fallback
both Rust bridges resolve to, so the registry must always be able to hold it
whatever a caller has already registered.

Every insertion path checks the bound before it mutates anything. A refused
configuration therefore leaves the registry exactly as it was, rather than
holding part of a logger set that was not accepted.

### Bounded dispatch

Loggers share a **process-wide, fixed-size pool of dispatch shards** instead of
each owning a worker thread. The pool holds at most four shards
(`DEFAULT_SHARD_COUNT`), clamped to the machine's available parallelism in
`[1, 4]`: a one-core machine need not spend four threads on logging, and a
64-core machine must not spend sixty-four.

Thread count is now a function of the machine rather than of the logger count,
which is the property the issue asked for. A logger is assigned a shard
round-robin at construction and keeps it, so each logger's records stay FIFO on
a single queue. The pool records the process that owns its workers. After a
`fork`, both inherited logger instances and newly constructed loggers detect
the changed process ID and bind to the child's pool before dispatching; the
parent's worker queues are never reused in the child.

Each shard bounds work twice:

- a **bounded job queue**, so a burst cannot grow memory without limit. Its
  depth is `1,024 × shard count`, so the pool scales with the machine while any
  one shard's depth stays proportionate to the pool it belongs to;
- a **pool of permits**, one per free in-flight slot, so a slow handler cannot
  turn a busy logger's backlog into an unbounded one. A bounded queue alone
  bounds memory but not latency: a logger that keeps logging while its handler
  is stuck would keep the queue full forever and never notice.

A permit travels *inside* the job it authorizes and is released from its own
`Drop`, after the handlers have run — or while a panicking handler unwinds.
Attributing the release to `Drop` rather than to a statement after the handler
loop is what makes the accounting hold on the panic path too.

Both refusals are returned to the caller, which counts the drop and warns about
it through the existing rate-limited warner. **The logging call itself never
waits**, which preserves the existing contract that a slow handler degrades
delivery rather than blocking application threads.

### Bounded resource failure

Pool startup is fallible. A process at its thread limit cannot have a pool, and
that is a resource refusal rather than a logic error, so `Shard::start`
propagates the operating system's `io::Error` and the logger constructors
return it as `DispatcherError` (a `RuntimeError` on the Python side).

The failure is deliberately **not recorded**. A process that has closed loggers
may well be able to start the threads it could not start before, so a later
caller retries instead of being permanently poisoned by a transient refusal.

A panicking handler does not stop a shard. Each job runs under `catch_unwind`,
so one logger's misbehaving handler cannot silence the other loggers sharing
that shard, and cannot kill the worker that serves them.

### The Rust bridge fallback is part of the bound

`log` and `tracing` resolve a target to a logger, and neither crate offers a
way to refuse a record. The bridges therefore resolve through the same bounded
lookup as every other caller, and a target the registry refuses falls back to
`root` rather than panicking inside a logging macro. The `log` bridge emits a
rate-limited diagnostic for a refused target; the tracing layer currently falls
back silently. Falling back rather than dropping matches what a logging bridge
must do — a caller cannot be handed an error from `log::info!` — while still
keeping the target space bounded.

## Consequences

- A process whose logger names come from unbounded input consumes bounded
  memory and a bounded number of threads, and reports the bound being reached
  rather than dying when it is.
- A caller can no longer use one logger per request, tenant or user identifier
  without eventually seeing `RuntimeError`. This is a deliberate behaviour
  change and the reason the users' guide documents the field-based alternative.
- Loggers sharing a shard share a failure domain: a slow handler now delays
  other loggers on the same shard. Each logger's own in-flight budget is capped
  below the shard's depth, so one logger cannot consume every permit and crowd
  the others out of the queue. That bounds *admission*, not *delivery*: a
  handler that never returns blocks the shard's single worker while still
  holding its permits, so the other loggers on that shard can exhaust their own
  allowances and drop records even with permits left in the pool. The shard
  count — derived from the machine, and never one on a machine that reports
  more — is what keeps that from stopping the whole process, so it is a real
  trade-off and the reason the shard count is not fixed at one.
- Records dropped at the in-flight or queue bound are counted and warned about,
  so pressure is observable through `logger.get_dropped()`.
- The Loom models keep the pre-change topology. Under `--cfg loom` a logger
  owns its single shard and joins it on drop, because `loom::model` runs its
  closure repeatedly in one process: a process-global pool would carry one
  iteration's threads into the next, and an idle shard blocked on a primitive
  Loom cannot schedule deadlocks the model. Every existing model and
  `tests/heavy/loom-models.txt` therefore remain valid unchanged, and the
  production pool is exercised by the ordinary tests.
- The registry's bound is asserted by unit tests on the boundary arithmetic
  plus an end-to-end test that walks the registry to the cap, which only became
  writable once the shared pool existed — before it, that test would have
  started ten thousand threads, which is the defect itself.

## Alternatives considered

- **Evict the least-recently-used logger at the cap.** Rejected: a logger is
  not a cache entry. Its handlers and level are caller-held state, so eviction
  redirects records silently. See the registry section above.
- **Weak ownership of registry entries, reaped when the caller drops its
  handle.** Rejected: `get_logger` without a store is a legal and idiomatic
  call, and the returned object is immediately collectable, so weak entries
  would be reaped almost always. A logger's identity would stop being stable
  across calls, which breaks the singleton contract the API promises and that
  stdlib-compatible code depends on.
- **Explicit registration: make callers declare their logger names up front.**
  Rejected as the only route: it is the right discipline for configuration and
  is already available, but `get_logger` is a dynamic lookup by design and
  demanding registration would break every caller that names a logger at the
  point of use. The bound plus documentation achieves the same end without
  removing the dynamic route.
- **A hand-rolled semaphore or injected spawn budget rather than the operating
  system's own thread limit.** Rejected as speculative: it would invent a
  limiter for a limit the operating system already enforces, and nothing in the
  crate reads a capacity hint that could overflow. Propagating the spawn error
  discharges the "resource exhaustion without panicking" requirement directly.
- **Keep one worker per logger and bound only the registry.** Rejected: it
  bounds the wrong thing. The registry cap is 10,000, which at one thread each
  is still ten thousand threads — the amplification the issue exists to remove.
- **Shard the queue by logger hash, one shard per core.** Considered in the
  form of "let available parallelism set the count", and rejected for the
  unbounded case: `available_parallelism` reported 64 on a machine where
  logging needs four threads, so the pool is clamped. Machine size sets the
  ceiling, not the number.

## References

- Issue #432 and its folded findings `892a59c0`, `a25c11d3`, `b721c5d5` and
  `193b1941`.
- `rust_extension/src/manager/validation.rs` — the name rules.
- `rust_extension/src/manager/registry.rs` — the registry cap.
- `rust_extension/src/logger/dispatcher.rs` and
  `rust_extension/src/logger/dispatcher/shard/` — the shared pool.
- `docs/users-guide.md` — the caller-facing naming guidance.
- ADR 004 (batching optimizations in consumer threads) — the equivalent
  single-consumer design for the file handler.
- ADR 006 (environment seam taxonomy) — the seam discipline the dispatch
  worker's thread, channel and lock primitives follow.
