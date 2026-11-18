# Architectural decision record (ADR) 0009: release collection locks before dropping user resources

## Status

Accepted — 2026-11-17: a collection guard covers mutation only. Anything the
mutation displaces leaves the guarded scope and is destroyed after the guard is
released.

## Date

2026-11-17

## Context and problem statement

`FemtoLogger` publishes its handler and filter collections behind
`parking_lot::RwLock`. Removing an entry is not a pure data-structure edit: the
removed `Arc` may hold the last reference to a Python object, and dropping it
runs that object's finalizer on the calling thread. A finalizer is arbitrary
user code, and user code logs. `get_logger(name)` then takes the manager lock
and `logger.info(...)` takes this logger's handler lock for reading.

`parking_lot`'s `RwLock` is not reentrant, so a finalizer that re-enters a
collection the caller still holds for writing deadlocks the thread. Reported as
issue #448 for `clear_handlers`, the same shape appeared in `remove_handler`,
`remove_filter`, `clear_filters` and the displacement helpers behind runtime
mutation, and in the manager's reset and runtime-replacement paths, where the
dropped value can be a logger, a handler or a filter.

The hazard is not limited to Python. A native handler's destructor may flush or
join its worker thread, and a worker that needs the same lock deadlocks the
same way. Python only makes the re-entry easy to observe.

## Decision

Adopt one lifecycle rule for every logger and manager collection:

1. A guard covers the mutation of its collection and nothing else.
2. Values removed or displaced by that mutation leave the guarded scope intact
   — `std::mem::take` for a whole collection, `Vec::remove` for one entry,
   `std::mem::replace` for a swap — and are destroyed after the guard ends.
3. A Python entry point may keep the destroy step inside `Python::attach` once
   the guard has gone, so the decref and its finalizer run immediately rather
   than being queued for the next attach.
4. The manager takes the values it is about to destroy out of `MANAGER` under
   the lock, drops the guard, and only then drops them. When the API promises
   to return displaced state to its caller, the guard ends before the return.
5. A fallible batch that has already built values never destroys its partial
   result under the guard. It either drops the partial result after releasing
   the guard, or hands it back with the error so the caller's unlocked failure
   path can. `stage_loggers` takes the first form. `register_missing_ancestors`
   takes the second by returning an `AncestorRegistrationError` that owns the
   ancestors built before the failure; a bare `Result<_, PyErr>` could not,
   because the error value would be forced to drop the partial batch in place.

The rule is stated as a rule, not as five fixes, because the next collection
mutation is the one that will reintroduce the defect.

## Consequences

- Removal helpers allocate a local `Vec` or `Option` instead of mutating and
  returning inside one statement. The cost is one move, paid only on removal.
- Displaced state is dropped by whoever receives it, which is already the
  observable contract of `replace_runtime_state` and of configuration commits.
- The rule needs proof that does not depend on a hang: dropping under the lock
  must fail a test quickly rather than wait forever. A probe whose `Drop`
  records `try_write` on the owning lock, and a child interpreter that prints a
  marker only after the re-entrant log call returns, both fail fast under a
  bounded timeout.
- Reentrant locks were rejected: `parking_lot` offers no reentrant `RwLock`,
  and one would trade a deadlock for an inconsistent read of a half-mutated
  collection.
- Dropping the values on a background thread was rejected: it makes collection
  timing non-deterministic and leaves user code holding resources after the
  call that removed it has returned.

## References

- Issue #448, "Drop removed handlers and filters only after releasing
  collection locks".
- [ADR 0007](adr-0007-bounded-logger-cardinality-and-shared-dispatch.md), which
  introduced the shared dispatch pool whose workers these destructors may wait
  on.
