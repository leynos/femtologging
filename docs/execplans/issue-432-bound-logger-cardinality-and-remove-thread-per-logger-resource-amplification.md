# Bound logger cardinality and remove thread-per-logger amplification (issue 432)

Status: IN PROGRESS.

## Purpose and big picture

`get_logger(name)` retains every distinct name for the life of the process, and
every `FemtoLogger` starts a dedicated worker thread with a 1 024-slot queue. A
caller that derives logger names from request, tenant, user, or plug-in
identifiers therefore converts attacker-influenced input into monotonically
increasing memory and OS-thread use.

After this change a caller can still call `get_logger`, the module-level
convenience functions, and the Rust `log`/`tracing` bridges with arbitrary
names, but the process's resource use stays bounded: names are validated
against a byte budget and a segment budget, the registry refuses new names once
a hard cap is reached, and loggers share a small fixed pool of dispatch threads
instead of each owning one thread.

## Conformance basis

- Issue #432 (acceptance criteria in the issue body), including the folded
  findings `892a59c0`, `a25c11d3`, `b721c5d5`, `193b1941`.
- `docs/adr-006-environment-seam-taxonomy.md` (environment-access policy) and
  `docs/adr-005-four-tier-python-lint-architecture.md` (lint gates).
- `AGENTS.md` (code style, 400-line file limit, en-GB-oxendict spelling).
- Repository conventions: `rust_extension/src/sync.rs` is the only place that
  names threads, channels and locks.

## Constraints

- No new third-party dependency. The shared pool, its fallible initialization
  and its drain-on-drop barrier use `std`, `crossbeam_channel`, `once_cell` and
  `parking_lot` only.
- `rust_extension/src/sync.rs` stays the single seam for thread, channel and
  lock primitives; `--cfg loom` builds must keep compiling and the heavy lane's
  models must still pass.
- Every module keeps its `//!` header; no file exceeds 400 lines.
- Errors are returned, not panicked: registry saturation and thread start-up
  failure surface as Python exceptions (or `Err` on the Rust API).
- No ambient environment reads (`std::env::*` remains banned).
- Existing behaviour that must not regress: per-logger FIFO delivery, flush
  acknowledgement, drop counting with rate-limited warnings, drain on drop,
  parent propagation, `reset_manager()` semantics, and the `"root"` fallback in
  the Rust bridges.

## Tolerances (exception triggers)

- Stop and escalate if the shared pool cannot preserve per-logger FIFO order
  while also bounding in-flight work.
- Stop and escalate if the loom models in `rust_extension/tests/heavy/` cannot
  be kept passing without weakening the design.
- Stop and escalate if a documented public API must break beyond the two new
  error cases (`ValueError` for invalid names, `RuntimeError` at the cap).
- Stop and escalate if any gate (`make lint`, `make typecheck`, `make test`)
  cannot be brought green with a scoped fix.

## Risks

- The dispatch pool introduces a new shared resource; a panic inside a job must
  not poison a shard. Mitigation: wrap each job in `catch_unwind` and keep the
  shard loop alive.
- Drain-on-drop could deadlock if the drop path holds a lock the shard needs, or
  if it is taken while the GIL is held. Mitigation: enqueue an acknowledgement
  job and wait on it with the existing 2 s flush timeout, detached from the GIL.
- Job channels are global; a single slow handler now blocks other loggers on the
  same shard. Mitigation: four shards (clamped to available parallelism), a
  per-logger in-flight bound equal to the previous per-logger queue capacity,
  and a documented trade-off.

## Verification plan

Obligations and their evidence:

1. Name validation rejects bad names on every creation path. Artefact: Rust
   unit tests in `manager.rs` and a Python test for the `FemtoLogger(name)`
   constructor. Evidence: `ValueError`/`PyValueError` with a bounded,
   control-escaped name in the message.
2. The registry never exceeds its cap, is not partially mutated by a rejected
   configuration, and reports `RuntimeError` at the cap. Artefact: Rust tests
   and `tests/test_logger_cardinality.py`.
3. Registry cardinality no longer implies thread cardinality. Artefact:
   `/proc/self/status` `Threads:` sampling in the same Python test after
   creating many loggers; skipped off Linux.
4. Per-logger FIFO order and flush acknowledgement survive the pool. Artefact:
   the existing lifecycle tests plus new shard tests in
   `rust_extension/src/logger/dispatcher_tests.rs`.
5. A panicking handler does not silence later records. Artefact:
   `catch_unwind` test in the dispatcher tests.

Non-vacuity: each test asserts an observable difference (an exception, a
registry count, a thread count, or a delivered message) that must change if the
corresponding implementation is reverted.

## Plan of work

1. Add bounded name validation and a registry cap in
   `rust_extension/src/manager.rs`, applied to `get_logger`, `stage_loggers`,
   `lookup_existing_logger`, the `FemtoLogger(name)` constructor, and the
   configuration staging/commit path. Tests at each boundary.
2. Add `rust_extension/src/logger/dispatcher.rs` and
   `rust_extension/src/logger/dispatcher/shard.rs`: a lazily started, fallible,
   fixed-size pool of dispatch shards. Move record processing out of
   `worker.rs`, keep per-logger FIFO by enqueueing to one shard per logger,
   bound in-flight work per logger with a permit pool, and replace the
   per-logger thread handle with an acknowledgement barrier for `Drop`. Under
   `--cfg loom` the shard is owned by the logger rather than drawn from the
   pool, and `Drop` stops and joins it instead of waiting on a barrier.
3. ~~Add `rust_extension/src/logger/spawn_budget.rs`~~ — dropped as
   speculative; see discoveries for 2026-10-16. The fallible pool start
   discharges the "handle resource exhaustion without panicking" criterion.
4. Align the public routes (`convenience_functions.rs`, `log_compat.rs`,
   `tracing_compat/mod.rs`) with the bounded policy; rate-limit the bridge's
   lookup-failure diagnostic.
5. Add stress tests (Python and Rust) and document naming and capacity in
   `docs/users-guide.md` plus ADR 007.

Milestone 2 detail:

- [x] `dispatcher/shard.rs`: the job queue, the permit pool, the worker loop,
      `Admit`/`wait_for_drain`, and `catch_unwind` around each job.
- [x] `dispatcher.rs`: the pool, round-robin assignment, `DispatcherError`,
      `shards_for` clamp, and the `POOL` `Lazy<OnceCell<..>>`.
- [x] `worker.rs`: construction resolves the pool (fallible) and stores the
      shard; `assembled` is shared by both `#[cfg]` arms.
- [x] `producer.rs`: `send_to_local_handlers` admits to the shard and counts a
      refusal as a drop; `flush_handlers_blocking` waits on the barrier first.
- [x] `mod.rs`: the `QueuedRecord` payload, the ungated
      `From<DispatcherError> for PyErr`, and the new `Drop`.
- [x] Seam: `spawn_named` and `recv_timeout` added; `recv_either`/`Either`
      and the Loom channel's `selectors`/`wake_on_change` removed as dead.
- [x] Loom arm compiles as a distinct construction path; `--no-default-features`
      lib type-checks clean.
- [x] `lib test` compiled after the admission outage lifted; the two real
      errors it had been hiding are fixed (see discoveries, 2026-10-16).
- [ ] All feature lanes checked; test targets re-checked after the edits
      below.
- [ ] Manager unit tests still to run (`--features python`).

## Progress

- [x] Rename branch, set the Lody session title, gather repository context.
- [ ] Milestone 1: bounded names and registry cap.
- [ ] Milestone 2: shared dispatch pool.
- [ ] Milestone 3: dropped as speculative (see discoveries).
- [ ] Milestone 4: public routes aligned, stresses, documentation, ADR.

Milestone 1 detail:

- [x] `MAX_LOGGER_NAME_BYTES` / `MAX_LOGGER_NAME_SEGMENTS` plus shape and
      control-character rules, in `rust_extension/src/manager/validation.rs`.
- [x] `MAX_LOGGERS` cap enforced on the vacant path in `get_logger`, before
      `ensure_registry_capacity` and again under the write lock in
      `commit_staged_loggers`.
- [x] `runtime_mutation::prepare_commit` checks capacity for the whole impacted
      set before fetching any logger.
- [x] `FemtoLogger::new` validates, making the constructor fallible.
- [x] Crate compiles with no warnings in the `--no-default-features` lane
      after the fallible-constructor change; 48 test call sites updated with
      `expect(...)`, then `cargo fmt` on the changed files.
- [ ] Remaining feature lanes (`python`, `log-compat`, `tracing-compat`, all)
      still to check.
- [ ] Manager unit tests still to run (`--features python`).
- [x] End-to-end cap test written (`tests/test_logger_cardinality.py`); it now
      walks the registry to `MAX_LOGGERS`, which the shared pool made possible.

## Surprises & discoveries

- 2026-10-14: the ledger's "hard cap 10 000 with `RuntimeError`" cannot hold as
  written: capacity checks that pre-allocate reject excess loggers as
  `MemoryError` (`HandlerError::Capacity`), and the independent per-handler
  capacity is fed from the logger count. A test that demanded `RuntimeError`
  for a 10 000-name configuration would contradict the capacity contract.
  Resolved in favour of the ledger with the cap semantics adjusted (see
  Decision log).
- 2026-10-14: `docs/execplans/issue-470-execute-loom-models.md` is not what the
  file name suggests; read `docs/execplans/README`-adjacent files before
  assuming a naming convention.
- 2026-10-16: the 400-line rule in `AGENTS.md` is gate-enforced, not stylistic.
  `make lint-rust` runs `whitaker --all`, and the Whitaker suite's
  `module_max_lines` lint defaults to exactly 400 (`src/config.rs` in
  `leynos/whitaker`). Adding the registry bound pushed `manager.rs` from 381 to
  589 lines, so the module was split into
  `manager/{validation,registry,manager_tests}.rs` behind a re-exporting
  facade. Splitting is therefore mandatory, not optional tidying: any file that
  crosses 400 lines fails the gate.
- 2026-10-16: the end-to-end cap test cannot be written before Milestone 2.
  Registering `MAX_LOGGERS` distinct loggers currently starts 10 000 OS threads
  — which is the defect itself. The Python cardinality test that walks the
  registry to the cap must land after the shared dispatch pool exists, or it
  will exhaust the machine instead of asserting a bound. The boundary
  *arithmetic* is unit-tested now (`registry_has_room`, `named_logger_count`)
  so the cap logic is covered without materializing the loggers.
- 2026-10-16: `runtime_mutation::collect_impacted` extends its set with
  `self.loggers.keys()`, so a runtime mutation really can name a logger that
  does not exist yet and cause `get_logger` to register it. The capacity
  pre-check in `prepare_commit` is load-bearing, not defensive.
- 2026-10-16: splitting `manager.rs` silently dropped test coverage. The
  original `mod tests` had `mod lookup` gated on `python` and `mod log_compat`
  gated on `log-compat`; the first draft of the split gated the whole test
  module on `python`, which would have stopped the flush-sweep tests running in
  the `log-compat` lane. The test module is now ungated, with per-test gates,
  so each lane keeps exactly the coverage it had.
- 2026-10-16: the `manager.rs` split needed re-exports that are easy to get
  wrong in a way only a compile catches: `registry_has_room_for_test` was
  re-exported from the facade but only ever reached as `super::registry::…`
  from inside the test module, producing an unused-import warning. Warnings are
  errors under `RUSTFLAGS=-D warnings` in the gates, so the re-export was
  removed.
- 2026-10-16: the entry API cannot coexist with a borrow of the map. Counting
  `named_logger_count(&mgr)` inside the `Entry::Vacant` arm fails to
  borrow-check (E0502) because the arm holds a mutable borrow of `mgr`; the
  count is taken before the `match` instead.
- 2026-10-16: `config/build.rs` does not need its own capacity pre-check.
  `commit_staged_loggers` already re-checks the same name set under the write
  lock and inserts nothing when it refuses, and no state changes in between, so
  a second check there was dead weight. The pre-check in
  `runtime_mutation::prepare_commit` is *not* redundant — it guards a different
  insertion path (`get_logger` on a not-yet-registered name).
- 2026-10-16: the "resource exhaustion without panicking" criterion is about
  `thread::spawn`. Re-read against the issue body, the requirement is that
  "`thread::spawn()` failure may panic"; there is no preallocating capacity
  helper and no `HandlerError::Capacity` variant in the crate (the only
  `Capacity` names are `BatchConfigError::ZeroCapacity` in the file handler).
  So the whole criterion is discharged by making the pool's start fallible and
  returning an error instead of unwrapping, which `Shard::start` already does
  through `std::thread::Builder::spawn`. A planned `spawn_budget.rs` with
  injected capacities was therefore dropped as speculative: it would have
  invented a limiter for a limit the operating system already enforces, and
  nothing in the crate reads a capacity hint that could overflow.
- 2026-10-16: the Loom models cannot share one process-global dispatcher pool.
  `loom::model` runs its closure repeatedly and a `Lazy` static would survive
  between iterations, so a shard thread spawned in iteration *n* would be
  waiting on a channel that iteration *n+1* writes to, which Loom reports as a
  cross-iteration state violation (and an idle shard blocked on an empty
  channel is a deadlock Loom will not schedule). The dispatcher is therefore
  *owned* under `--cfg loom` — one single-shard dispatcher per logger, joined
  on drop, which is exactly today's thread topology, so all six models and
  `tests/heavy/loom-models.txt` stay valid and unchanged. Production keeps the
  shared pool; the cardinality bound is asserted by non-Loom tests. See the
  Decision log.
- 2026-10-16: the in-flight bound is a pool of *permits*, not a queue length.
  A bounded queue alone bounds memory but not latency: a logger that keeps
  logging while its one handler is stuck would keep the queue full forever and
  never notice. Each shard therefore also holds one `Permit` per free slot, and
  `admit` takes a permit before it queues. A permit travels *inside* the job it
  authorizes and is released from its own `Drop`, so the accounting holds on
  the path where a handler panics and unwinds through the job as well as on the
  ordinary path; a drop after the loop would be skipped on the panic path.
- 2026-10-16: the drain barrier is sent with `send_timeout`, not `try_send`. A
  shard at the end of a burst has a full queue but a working worker, and
  refusing the barrier there would report a flush that succeeded as a failure.
  The bounded `send_timeout` waits for room and still cannot hang, because the
  worker is draining that same queue.
- 2026-10-16: the per-logger worker's two-channel `select!` was the seam's only
  caller of `recv_either`/`Either`, and the Loom channel's `selectors` and
  `wake_on_change` existed only to serve that helper. All three are removed:
  the shard has one queue, and a barrier is an ordinary job on it. Leaving them
  would have been dead code that the `-D warnings` gates reject anyway.
- 2026-10-16: a Loom model's `Drop` must not wait on a barrier taken from the
  logger's own shard, because the reachable path is
  `logger → dispatcher → shard` and it is logically impossible to take the
  shard out of the struct first without restructuring the type. `Drop` calls
  `shut_down()` under Loom (stop + join) and the barrier under the pool.
  Joining is also the stronger guarantee, so no model loses coverage. This is
  why the non-Loom wait had to become `#[cfg(not(loom))]`.
- 2026-10-16: `admit` returning `Result<(), QueuedRecord>` makes `expect`
  unusable at its call sites: `expect` requires `E: Debug`, and `QueuedRecord`
  holds `Arc<dyn FemtoHandlerTrait>`, which is not `Debug`. The tests assert
  with `is_ok()` instead. Deriving `Debug` on a public type purely to satisfy a
  test's assertion macro would have widened the public surface for the test's
  own convenience.
- 2026-10-16: a compile-admission outage verifies nothing, so a lane that
  could not run is not evidence. While the supervisor was unhealthy the
  `lib test` target never compiled; when admission returned it reported `E0277`
  twice at `logger_tests_shutdown.rs` for the reason above. The earlier clean
  `--no-default-features` run had covered the lib target only, and the two
  error sites live in test modules that lane never builds. Any claim of
  "compiles clean" must name the lane and the target set.
- 2026-10-16: the `log` bridge had no guard against its own targets, and adding
  the lookup diagnostic would have turned that into a feedback loop. The
  adapter is the global Rust logger, so
  `log::warn!(target: "femtologging.log_compat", …)` inside `resolve_logger`
  would be routed back into `resolve_logger`. The triggering condition makes it
  worse rather than benign: the warning fires because the caller is streaming
  many distinct targets, so an unguarded warning adds a record to the very
  stream it is warning about. The tracing bridge has had `should_ignore_target`
  all along; the `log` bridge now has the same rule (`femtologging` and
  `femtologging.*`), applied in `log()` before the target is resolved. Caught
  by reading the code while compilation was blocked, not by a gate — the gate
  that would have caught it is a running test.
- 2026-10-16: a `#[test]` in the `log_compat`/`tracing_compat` unit modules
  cannot saturate the registry. The registry is process-global and the unit
  binary runs its tests in parallel, so a case that walked to `MAX_LOGGERS`
  would refuse every neighbouring test's `get_logger` call. Both new bridge
  cases therefore reach the refusal arm through name validation (over-budget
  name, control character, too many segments) rather than through the cap. The
  cap itself is covered end-to-end by `tests/test_logger_cardinality.py`, which
  owns its process.
- 2026-10-23: `make test` found two real defects in the new dispatcher, both
  invisible to `lint-rust` because both compile and both are wrong:
  1. **Panic isolation was per record, not per handler.** `run_worker` wrapped
     `catch_unwind` around the whole `handle_job`, so a panicking handler
     aborted the loop and the *remaining handlers of the same record* never saw
     it. The module doc promised "one logger's handler cannot silence the
     loggers sharing the shard", and that held — but not the stronger promise a
     reader would infer, that a handler only harms itself. The test
     `a_panicking_handler_does_not_silence_later_records` caught it: its
     collecting handler lost `"before panic"` because the panicking handler
     ran first on the same record. Fixed by catching per handler inside the
     loop, with the outer catch kept as a backstop for panics outside a handler
     call, and the two messages made distinct so a log reader can tell which
     happened.
  2. **`one_shard_refuses_records_beyond_its_bound` never reached the bound.**
     A pool shard's depth is `RECORDS_PER_LOGGER * shard_count` — 1024 at the
     minimum — so holding one in-flight record left 1023 free permits and the
     second `admit` succeeded. The test asserted a refusal it had no way to
     provoke. Fixed by starting a `Shard` directly at depth 1: depth is the
     only input the bound depends on, so the refusal path is the same one and
     the test is now deterministic rather than racy.
- 2026-10-23: the integration test `log_formats_message::case_3` failed once
  the lib tests passed. It built a logger named `""` with an empty message and
  expected `" [INFO] "`, and the new validation refuses an empty name. The case
  had conflated two independent things: an empty *message* is ordinary and an
  empty *name* is not. The message case is kept with a real logger name, and a
  separate `an_empty_logger_name_is_refused` test pins the refusal at the public
  `FemtoLogger::new` boundary — the manager's unit test already covered it,
  but only one layer down, and this is the contract a caller meets first.
- 2026-10-23: **the most serious defect of the task**, and the one no lint could
  see: `run_worker` used `while let Ok(job) = recv_timeout(&jobs, IDLE_TICK)`.
  `recv_timeout` returns `Err(Timeout)` when the queue is quiet, so that loop
  treated *two seconds of idleness as shutdown*: every pool worker exited the
  first time the process logged nothing for the idle tick, and the damage
  appeared on the next flush, whose barrier waited for a worker that no longer
  existed. Every `flush_handlers()` then returned `false`. Why it was invisible
  until now: individual tests log, flush and finish well inside two seconds, and
  `make lint-rust` compiles but never runs. Under `--cfg loom` the seam's
  `recv_timeout` ignores its bound and blocks, so the Loom models cannot time
  out either — the lane that would otherwise have found it is blind to it by
  construction. Fixed by matching the error: `Timeout` continues the loop,
  `Disconnected` ends it. Added `an_idle_worker_keeps_answering`, which sleeps
  past `IDLE_TICK` and then flushes — the only test in the suite that
  deliberately idles a worker, and the reason the tick's meaning is now pinned.
- 2026-10-16: `log`'s global max level defaults to `Off`, so a bridge test
  written as a plain `#[test]` passes vacuously: `log()` returns at
  `is_enabled_by_global_max` before reaching the ignore list or the registry.
  Both new cases take the existing `log_max_level` fixture and use `#[rstest]`
  to do so, and each says in its doc comment that the fixture is required
  rather than decorative — a vacuous pass here would silently retire the
  coverage the test was added for.

## Decision log

- 2026-10-23: `test_stream_builder_applies_callable_formatter` is diagnosed
  but **not fixed** in this change. Rationale: it exercises a defect this
  change did not introduce and cannot reach, and fixing it would widen the diff
  past the issue. Evidence, so the next person does not re-derive it: the test
  builds only a `StreamHandlerBuilder`, no `FemtoLogger`, so no dispatcher
  shard is involved; `stream_handler.rs` and `formatter/python.rs` are
  untouched by this branch (`git diff` empty). The defect is that `close()`
  waits on a done-signal the worker only sends after it has formatted every
  queued record, and formatting a Python-callable formatter needs the GIL
  (`Python::attach`) while the calling thread still holds it inside `close()`,
  so the wait cannot be satisfied until `close()` returns. Measured directly:
  with a Python callable, `close()` burns its full one-second timeout on
  200/200 runs (p50 1.0003 s); with the Rust default formatter it returns in
  0.1 ms. The record itself is still written in 60/60 runs once the
  interruptible wait releases the GIL, which is why the test usually passes.
  Under load around 25 it fails about one run in ten: `capfd` then reads the fd
  before the write lands, capturing `''` or a torn `'allable:hello'` — the two
  shapes actually observed. No worker thread leaks: `/proc/self/status` thread
  count is unchanged across ten handler closes. The correct fix is to release
  the GIL around the wait in `close()` and to have the worker acknowledge each
  command, which is stream-handler work and belongs in its own change.

- 2026-10-14: reuse the existing `HandlerError::Capacity` path rather than
  exporting a new exception type. Rationale: the project already maps handler
  capacity refusal to an exception subclass; a third Python-visible error for
  the same class of event adds surface without adding information.
- 2026-10-14: the registry cap is enforced only on the vacant insertion path
  and never for `"root"`; an existing logger's identity is always preserved.
  Rationale: the issue's acceptance criteria forbid unbounded growth, not
  re-resolution of already-named loggers.
- 2026-10-16: `DispatcherError` is public, not `pub(crate)`. Rationale:
  `FemtoLogger::with_parent` is a public Rust API and is now fallible, so
  hiding the error type would have made the constructor's promise unstateable —
  and Rust warns (`private_interfaces`) rather than silently allowing it. It is
  re-exported from the crate root beside `FemtoLogger`.
- 2026-10-16: shard count is clamped to `[1, 4]` from the machine's available
  parallelism, rather than being a plain constant. Rationale: a small machine
  need not spend four threads on logging, and a large one must not spend
  sixty-four; the cap is what makes pool size a function of the machine rather
  than of logger count.
- 2026-10-16: the Loom arm keeps one shard per logger, which is the pre-change
  thread topology. Rationale: the six existing models and `loom-models.txt`
  then stay valid unchanged, so the Loom lane keeps testing the routing,
  ordering and handler-invocation properties it already asserted while the
  production pool is exercised by the ordinary tests.
- 2026-10-16: the `log` bridge distinguishes a saturated registry from any
  other `RuntimeError` by matching a shared `manager::REGISTRY_FULL_MARKER`
  constant, rather than by repeating the message text or by giving the cap its
  own exception class. Rationale: the marker lives beside the message that
  embeds it, so the two cannot drift, and a third Python-visible error type for
  one condition would widen the public surface to serve a diagnostic.
- 2026-10-16: the bridge's lookup-failure diagnostic is rate-limited through
  the existing `RateLimitedWarner`, matching the drop warning in
  `logger/producer.rs`. Rationale: the failure it reports is a property of a
  *stream* of targets, not of one record, so one warning per affected record
  would make the diagnostic's own cost proportional to the attack it describes.
- 2026-10-23: a panicking handler is caught per handler, not per record, and
  the worker keeps an outer catch as well. Rationale:
  `FemtoLogger::add_handler` lets independent owners attach handlers to one
  logger, so one of them panicking is that owner's bug. Catching around the
  whole record would make every other handler on the same record lose a record
  it was entitled to — a blast radius wider than the bug. The outer catch stays
  because the inner loop only covers handler calls; a panic in the shard's own
  bookkeeping must not end the shard for every logger assigned to it, and the
  two warnings are worded differently so the log says which case occurred.
- 2026-10-23: `Shard::admit` takes and returns `Box<QueuedRecord>` instead of
  a by-value `QueuedRecord`. Rationale: the queue's buffer is allocated at its
  full depth when the shard starts, so an inline record reserves the record's
  several hundred bytes in every slot of every shard — roughly 1.8 MB of empty
  slots for a four-shard pool at 4096 slots each. The box also keeps the
  refusal path allocation-free, which matters because that path runs when the
  process is already short of a resource, and it stops `Err` from carrying a
  432-byte payload. The alternative — boxing the exception and stack payloads
  inside `FemtoLogRecord` — was rejected here: they are pre-existing public
  fields, and changing their representation to satisfy a lint on the dispatcher
  would widen this change's blast radius well beyond the issue.

## Delivery log

- 2026-10-16: compile admission is **UNHEALTHY** and needs operator action:
  `build-limits-status` reports `capacity 4, held 3, UNHEALTHY` with
  `required recovery action: Restore manager connectivity and account for
  detached sccache requests; tokens remain held until the service is verified
  stopped and empty.`
  Every Rust gate (`lint-rust`, `typecheck`, `test`) is therefore blocked. Per
  the standing policy this is reported rather than worked around. The
  compiler-free gates all pass: `check-fmt`, `markdownlint`, `spelling`,
  `nixie`, and `lint-python`.
- 2026-10-16: the Rust bridge work is written but **not compiled**. It adds
  `log_compat_tests.rs` (the `log_compat.rs` test module moved out to keep the
  parent under 400 lines), a shared `REGISTRY_FULL_MARKER` so the bridge can
  tell a saturated registry from any other `RuntimeError` without duplicating a
  format string, a rate-limited lookup diagnostic, and one stress test per
  bridge. `cargo fmt` passes; `cargo check` could not run.

- 2026-10-16: implementation of milestones 1 and 2 is written but **not yet
  committed and not yet fully gated**. `cargo check --no-default-features` on
  the lib type-checks clean (0 errors, 0 warnings after the edits recorded
  above); the `--all-targets` run and every remaining lane are outstanding.
- 2026-10-23: admission is healthy again and `make lint-rust` has now run
  against the tree for the first time. Three rounds of real findings, each
  fixed before the next run:
  1. `E0716` in `log_compat_tests.rs`: `format_args!` borrowed a temporary that
     died at the end of the builder statement while the `Record` outlived it.
     Fixed by binding the `Arguments` to a `let` first.
  2. `unused import: registry::named_logger_count_for_test` in `manager.rs`:
     the re-export was gated `#[cfg(test)]`, so every lane without
     `tracing-compat` compiled it and never used it, and `-D warnings` refused
     it. Gated to `#[cfg(all(test, feature = "tracing-compat"))]`, which is the
     only configuration whose tests call it.
  3. `CountingHandler`/`with_first_signal` in `logger_tests_helpers.rs` and
     `dispatcher/shard.rs`'s `clippy::large_enum_variant` +
     `clippy::result_large_err`. The helper's only caller was the deleted
     per-logger `worker_thread_loop_shutdown_exits_under_load`, so it was dead
     code and removed. The two clippy findings were real design points, not
     noise: a shard's queue buffer is allocated at full depth, so a 440-byte
     `Job` put ~1.8 MB of empty slots in every shard; `admit` now takes and
     returns `Box<QueuedRecord>`, so a refusal allocates nothing.
- 2026-10-23: `make lint-rust` and `make typecheck` both exit 0, and
  `tests/test_logger_cardinality.py` passes 16/16, including
  `test_thread_count_does_not_track_logger_count` — the issue's central claim,
  measured from `/proc/self/status`. The lib suite alone reports
  `487 passed; 0 failed; 1 ignored` after the idle-tick fix. `make test` itself
  has not yet completed in one piece. Two harness-tracked background runs were
  killed at their time limit while inside the trybuild UI suite; under load
  around 20 on 24 cores each UI case compiles through the admission pool, so
  that one target now dominates the gate. The suite is therefore being run
  detached from the harness under `setsid`, with its output going to the `/tmp`
  log, and will be read from the log rather than a wrapper's exit code. No
  orphans were left by either killed run: no `cargo`, `rustc` or `make` process
  in this worktree remained, and the worktree's target lock was free before the
  rerun, so the detached run is not duplicated work.
- 2026-10-16: the last `--all-targets` run could not finish. Two targets were
  refused by the compile-admission supervisor:
  `build-limits: the supervisor refused: unhealthy`. A subsequent
  `build-limits-status` reports
  `cannot query the supervisor at /run/user/1000/buildlimits/supervisor.sock:
  [Errno 111] Connection refused`,
  so the supervisor is not running. Per the standing policy this is not worked
  around: no retry, no duplicate build, no `RUSTC_WRAPPER` override. The
  `sccache` socket in the same directory is still present, so the cache server
  is a separate process and unaffected.
- 2026-10-23: **the Loom lane had never actually run, and it was broken in four
  ways.** The earlier entries list it as outstanding; when it was finally
  executed, `make test`'s four ordinary feature lanes passed while the
  `--cfg loom` build did not compile at all. This is the failure mode a single
  unrun lane hides: four lanes green made the branch look gated.
  1. `E0277` in `worker.rs`: the Loom arm passed `Shard::start`'s
     `std::io::Error` through `?` into a `DispatcherError`. Loom's thread
     builder cannot fail, so there is no pool whose partial start an error
     could describe; the arm now `expect`s, which is what its own doc comment
     in `sync.rs` already said about Loom's builder.
  2. `E0004` in `shard.rs::admit`: the `Job` match did not cover `Job::Stop`,
     which exists only under `--cfg loom`.
  3. `RUSTFLAGS="--cfg loom" cargo clippy --all-targets -- -D warnings`, which
     is CI's own command, then found dead code the heavy lane cannot see
     because it is an integration test: `Dispatcher`, its impl, `test_support`
     and both `idle_tick_for_test` re-exports are all unreachable under Loom.
     Each is now gated `not(loom)`, with the reason recorded at the gate.
  4. `sync_tests.rs` lost its file-level `use super::*;` when the `ordinary`
     module was deleted, so the surviving `loom_arm` globbed nothing and could
     not resolve `bounded`, `spawn` or `TrySendError`. It now names those three
     imports explicitly.
- 2026-10-23: with the Loom build compiling, all four topology models failed
  with `Model exceeded maximum number of branches` while the two models that
  never construct a `FemtoLogger` passed. The cause was construction, not
  behaviour: `Shard::start` fills its permit pool with one channel send per
  slot, and under Loom every send is a scheduling point, so a
  `RECORDS_PER_LOGGER` (1024) shard spent the entire branch budget before the
  model asserted anything. `LOOM_RECORDS_PER_LOGGER = 8` now sets the depth for
  a model's shard — the models put at most three records in flight, and depth
  is not what they assert. All six models then pass in 36.76s.
- 2026-10-23: the local Loom run also needed the environment CI documents:
  without `PYO3_PYTHON` the released test binary links `libpython3.14.so.1.0`
  from a path the loader cannot see and exits 127 before any model runs.
  `PYO3_PYTHON` plus `LD_LIBRARY_PATH` pointing at the same uv interpreter
  fixes it locally; CI gets this from `setup-python` per the comment in
  `heavy-tests.yml`.
- 2026-10-23: with those fixes, `make fmt`, `make lint-rust`, `make typecheck`
  and the full detached `make test` all exit 0 — four cargo feature lanes plus
  `755 passed, 5 skipped` in pytest — and the Loom heavy lane reports
  `6 passed; 0 failed`.
- 2026-10-23: delivered as two commits — `2119087` (the implementation and its
  tests) and `39839c2` (the ADR, the guides and this plan) — pushed over SSH to
  `origin`, with the tracking ref refreshed and the upstream set. The remote
  head and the local `HEAD` both read `39839c2`, confirmed by `ls-remote`
  before the push was reported. Draft PR
  [#544](https://github.com/leynos/femtologging/pull/544) carries the issue
  number and `Closes #432`.
- 2026-10-23: the first `coderabbit review --agent` attempt was rate-limited
  ("Rate limit exceeded", 10 included reviews used, 2-minute wait). Retried
  after a randomized 45–90 minute sleep, per the delivery instructions.

## Outcomes & retrospective

Complete pending review. All eight acceptance criteria in the issue are met and
evidenced:

1. **Name bounds** — 255 UTF-8 bytes and 32 segments, enforced at the single
   point where a name becomes a registry key.
2. **Control characters** — refused, including `0x7F`.
3. **Cardinality policy** — decided and documented in ADR 007: a hard cap with
   validation, rather than weak ownership or eviction, because a logger is a
   stable component identity and evicting one would silently reset its level
   and handlers.
4. **Thread multiplier removed** — a fixed pool sized from available
   parallelism, measured flat against logger count in
   `test_thread_count_does_not_track_logger_count`.
5. **Exhaustion without panic** — `DispatcherError` to `RuntimeError`, with a
   later retry.
6. **One policy for every entry point** — convenience functions and both Rust
   bridges resolve through the same registry.
7. **Stress tests** — many unique names, over-long names, and
   high-cardinality bridge targets.
8. **Guidance** — the guides state that names describe stable components, with
   a do/don't example.

The lasting lesson is about gates, not code: four green feature lanes made a
branch look gated while the Loom lane had never run and did not compile. A lane
that has never been executed is not evidence, and the `-D warnings` clippy run
under `--cfg loom` catches dead code that the Loom heavy binary structurally
cannot — because it is an integration test, not a unit test build.
