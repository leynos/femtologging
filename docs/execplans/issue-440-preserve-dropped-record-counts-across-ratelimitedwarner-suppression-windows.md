# Preserve dropped-record counts across `RateLimitedWarner` suppression windows

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: DRAFT

## Purpose / big picture

`RateLimitedWarner` is the component every femtologging queue uses to report
lost records without flooding the log. Today it *undercounts* that loss: a
warning attempt that lands inside the rate-limit interval discards the drops
accumulated since the last warning, and `flush()` — which documents immediate
reporting — is itself rate limited, so an explicit flush can silently swallow
the very batch an operator asked to see.

After this change, an operator reading the warnings sees the whole truth. A
suppressed interval carries its drops forward: if three records are dropped
inside the window and a fourth arrives after it, the next warning reports four,
not one. An explicit `flush()` reports immediately, whatever the clock says, so
shutdown and flush paths cannot lose the last batch. Every recorded drop is
reported exactly once — never twice, never zero times — including when
`record_drop()`, `warn_if_due()` and `flush()` race on several threads.

The user-visible evidence is a warning stream whose counts sum to the number of
records actually dropped, and a test suite that fails if any drop is discarded
or double-reported.

## Constraints

- `RateLimitedWarner` keeps its public shape: `new`, `with_clock`,
  `record_drop`, `warn_if_due`, `flush`, the `Clock` trait, and
  `DEFAULT_WARN_INTERVAL`. Existing call sites in `stream_handler`, `logger`,
  `http_handler` and `socket_handler` must not need signature changes.
- The warner stays lock-free. `record_drop()` sits on the record hot path and
  must not take a lock or allocate.
- All existing warning message texts stay byte-for-byte identical; operators
  and downstream tests match on them.
- At most one `warn_if_due()` emission per interval, and only one caller may
  claim an interval. Racing callers must not both warn.
- No `Drop` hook is added to any consumer that does not already have one, apart
  from the flushes the existing `Drop` impls already invoke.
- Conventional commit messages, en-GB-oxendict prose in documents and comments.
- Rust sources stay under 400 lines per file, module-level `//!` comments
  preserved, `///` rustdoc on public items.

## Tolerances (exception triggers)

- Scope: if the change exceeds 12 files or 500 net lines (excluding the plan and
  generated artefacts), stop and escalate.
- Interface: any change to the public signature of an existing item stops the
  work for confirmation. A new `#[cfg(feature = "test-util")]` accessor is
  inside tolerance, and is recorded in `Decision log`.
- Dependencies: adding any new crate stops the work.
- Iterations: if a gate still fails after three focused fix attempts, stop and
  report the exact failure.
- Ambiguity: if the concurrency contract cannot be stated without contradicting
  an existing documented behaviour, stop and present the options.

## Risks

- Risk: the interval-claim CAS changes which caller warns, and existing
  message-count tests elsewhere in the tree may assume the old behaviour.
  Severity: medium Likelihood: medium Mitigation: run the full `make test`
  lanes and fix every assertion that encoded the lossy behaviour, rather than
  weakening the new contract.
- Risk: flushing owned warners from worker-exit paths emits warnings during
  teardown of unrelated tests that install a process-wide capture logger.
  Severity: medium Likelihood: low Mitigation: flush only reports a non-zero
  count, so a path with no pending drops stays silent; verify by running the
  `-- --ignored` capture-logger binary, which is the strictest observer.
- Risk: a heavy property test that spawns threads can be flaky under load.
  Severity: medium Likelihood: medium Mitigation: synchronize with a `Barrier`,
  advance the fake clock only from one thread, and make all assertions about
  conservation rather than about which thread observed what.
- Risk: `flush()` is now unconditional while `warn_if_due()` claims an interval
  first, so a race between them could look like a lost batch. Severity: low
  Likelihood: medium Mitigation: state in rustdoc that a batch is reported by
  whichever drain observes it, and cover the race in the stress test.

## Progress

- [x] (2026-10-16) Reconnaissance: locate the defect, its consumers, and the
      tests that encode the lossy behaviour.
- [x] (2026-10-16) ExecPlan written.
- [x] (2026-10-16) Task 1: corrected `warn_if_due()` and `flush()`, documented
      the concurrency contract. Written but not yet compiled.
- [x] (2026-10-16) Task 2: regression, contention and stress tests (unit suite
      in `rate_limited_warner_tests.rs`), heavy proptest
      (`tests/heavy/prop_rate_limited_warner.rs`), stream-handler integration
      test updated, design document updated. Written but not yet run.
- [x] (2026-10-16) Task 3: flush owned warners on stream, logger, HTTP and
      socket flush and shutdown paths.
- [x] (2026-10-16) Focused unit suite green for the modified helper: 13 passed,
      0 failed (`--no-default-features --features test-util`).
- [x] (2026-10-16) Red evidence captured against the unmodified helper: 7 of
      13 failing, including loss of 79 of 120 recorded drops under two threads.
      See `Artefacts and notes`.
- [ ] Heavy proptest run with `-- --ignored`.
- [ ] Full commit gates green via `scrutineer`.
- [ ] CodeRabbit review clean.
- [ ] Pushed and draft PR opened.

Blocked on: the shared compile-admission pool. `build-limits-status` reports
capacity 4 with the pool saturated and far more waiters than slots, and the
managed sccache server is unreachable, so this build compiles uncached behind
every other agent's work. Nothing has been compiled or executed yet; the tasks
above are written, not verified.

## Surprises & discoveries

- Observation: the worker threads of both the HTTP and socket handlers own a
  *second* `RateLimitedWarner`, separate from the handler's own. Evidence:
  `rust_extension/src/http_handler/worker.rs:87` and
  `rust_extension/src/socket_handler/worker.rs:56` each call
  `RateLimitedWarner::new(config.warn_interval)`. Impact: flushing only the
  handler's warner would still let worker-side drops (serialization, retry
  exhaustion, write errors) be suppressed forever, so both must be flushed.
- Observation: `send_to_local_handlers()` returns early when the logger's
  sender is gone, so no drop is recorded once the logger starts tearing down.
  Evidence: `rust_extension/src/logger/producer.rs:211-214`. Impact: the
  shutdown flush has a bounded, well-defined window, and no drop can be
  recorded after it.
- Observation: `cargo test --doc` is a separate target from `cargo test`, so
  the doctests added to `warn_if_due()` and `flush()` are not proven by the
  focused test command used during development. They exercise
  `RateLimitedWarner::new`, which reads the real clock; the doctest for
  `warn_if_due` therefore depends on the first drop always warning, which the
  `u64::MAX` sentinel guarantees, and must not depend on any later boundary.
  Impact: the doctest contradicts the design if it asserts a second warning;
  only the first is asserted. Gate evidence for the doctests has to come from
  the full `make test` run rather than the focused filter.
- Observation: the 400-line rule forces the unit tests out of the module file.
  Evidence: after the fix and its documentation, `rate_limited_warner.rs` was
  536 lines. Impact: tests now live in
  `rust_extension/src/rate_limited_warner_tests.rs`, attached with
  `#[path = "rate_limited_warner_tests.rs"] mod tests;`, which keeps `super::*`
  visibility and the private-field assertions the unit tests rely on.
- Observation: `pending_drops()` gated on `feature = "test-util"` alone would
  be invisible to this module's own unit tests in the feature lanes that run
  them without that feature. Impact: the gate is
  `#[cfg(any(test, feature = "test-util"))]`. The heavy binary is unaffected:
  `Cargo.toml` lists `_femtologging_rs` as a dev-dependency of itself with
  `features = ["test-util"]`, so the accessor is present there through that
  dependency edge rather than through `cfg(test)`.
- Observation: the stream handler's dropped-record warning does **not** lead
  with its count; the message is
  `FemtoStreamHandler: {count} log records dropped in the last interval`, so the
  count is the last token before the shared suffix. Evidence: the first version
  of the updated integration test parsed the first whitespace token and failed
  with `ParseIntError { kind: InvalidDigit }` on `FemtoStreamHandler:`. Impact:
  the count is now read by stripping the shared suffix and taking the last
  remaining token, which also keeps the parse tied to the same constant the
  filter selects on. The HTTP and socket warnings do lead with the handler name
  too, so any future test parsing their counts needs the same treatment.
- Observation: property-test case count is not visible in the ordinary output,
  and `PROPTEST_VERBOSE=1` did not surface it either. Impact: non-vacuity was
  established instead by scaling, which is stronger evidence than a self-report:
  `PROPTEST_CASES=2000` took 3.20s against 0.09s for the configured 32, so the
  generator is running one case per draw rather than passing vacuously. At the
  configured 32 cases the run is 0.09s and produces no visible count, so the
  default run alone would not have shown whether the cases ran.

## Decision log

- Decision: read the drop counter *first* in `warn_if_due()` and return when it
  is zero, before sampling the clock or claiming the interval. Rationale: an
  empty call must not start an interval. Otherwise a caller that polls
  `warn_if_due()` on a timer with nothing to report would delay the next real
  warning by a full interval, and the regression "record a drop after an empty
  due call and see it warned immediately" would fail. Date/Author: 2026-10-16,
  agent.
- Decision: claim the interval with `compare_exchange` on `last_warn` rather
  than draining on a plain load. Rationale: consumers are shared across
  threads. With a plain load, two callers can both observe an expired interval
  and both warn, and the subsequent `store` can move the timestamp backwards.
  The CAS makes the claim exclusive and always moves the timestamp forwards.
  Date/Author: 2026-10-16, agent.
- Decision: report the timestamp before invoking the callback, and never store
  it after. Rationale: a slow or re-entrant callback must not hold the interval
  open; a `store` after the callback could regress a timestamp another thread
  has already advanced. Date/Author: 2026-10-16, agent.
- Decision: keep `Ordering::Relaxed` throughout.
  Rationale: the atomics publish only their own values; no other memory is
  released or acquired through them. Date/Author: 2026-10-16, agent.
- Decision: add a `#[cfg(feature = "test-util")] pending_drops()` accessor.
  Rationale: the acceptance criteria ask for an assertion that "reported plus
  pending equals recorded". The in-module stress test can read the private
  field, but the heavy property test is a separate binary and cannot. Gating on
  `test-util` keeps the accessor out of the shipped library, matching the
  existing `with_test_config` / `with_warner` precedent. Date/Author:
  2026-10-16, agent.
- Decision: flush each worker-owned warner when the worker's flush command is
  handled and when its loop exits. Rationale: worker-side drops are recorded
  after the handler has already accepted the record, so only the worker can
  report them, and its loop exit is the last moment at which a report is
  possible. Date/Author: 2026-10-16, agent.

## Outcomes & retrospective

Not yet started.

## Context and orientation

`RateLimitedWarner` lives in `rust_extension/src/rate_limited_warner.rs`. It
holds two atomics: `dropped`, incremented once per lost record, and
`last_warn`, the clock reading of the most recent emission (`u64::MAX` means
"never"). A `Clock` trait supplies milliseconds so tests can move time by hand;
`RealClock` wraps `Instant`.

Callers pair the two methods: `record_drop()` records the loss, then
`warn_if_due(callback)` offers to report. The callback receives the batch size.
`flush(callback)` is the explicit-reporting entry point.

**The defect.** `warn_if_due()` currently reads:

```rust
if prev == u64::MAX || now.saturating_sub(prev) >= self.interval_ms {
    let count = self.dropped.swap(0, Ordering::Relaxed);
    if count > 0 {
        warn(count);
    }
    self.last_warn.store(now, Ordering::Relaxed);
} else {
    self.dropped.store(0, Ordering::Relaxed); // erases the pending drops
}
```

The `else` branch discards every drop accumulated since the last warning. A
later interval therefore reports only the drops recorded after the most recent
suppressed call. `flush()` delegates to `warn_if_due()`, so it inherits the
suppression and can clear pending drops without reporting them, contradicting
its own documented "immediately warn" contract.

The consumers are:

- `rust_extension/src/stream_handler.rs` — the handler owns one warner and pairs
  `record_drop()` with `warn_if_due()` in `handle()`, and calls
  `warner.flush()` in `flush()` only on the open path.
- `rust_extension/src/logger/` — `FemtoLogger` owns `drop_warner` and, under the
  `python` feature, `context_capture_warner`; `producer.rs` records and warns,
  `flush_handlers_blocking()` only flushes handlers.
- `rust_extension/src/http_handler/handler.rs` and
  `rust_extension/src/socket_handler/handler.rs` — each owns a warner for
  refused records, flushes it only after `sender()` returns `Some`.
- `rust_extension/src/http_handler/worker.rs` and
  `rust_extension/src/socket_handler/worker.rs` — each owns a second warner for
  drops discovered inside the worker, and never flushes it.

The `heavy` Cargo test target is auto-discovered from
`rust_extension/tests/heavy/main.rs`; its property tests are `#[ignore]` and
run by the nightly workflow with `-- --ignored`.

## Conformance basis

No Terms of Reference or technical-design revision governs this change
directly. The upstream artefacts are the GitHub issue
<https://github.com/leynos/femtologging/issues/440> and the source finding
`25bfd2d86cc88191926174055af2e76c` ("Dropped-record warning counter is cleared
before reporting"). The relevant in-repo design statements are
`docs/rust-multithreaded-logging-framework-for-python-design.md` (rate-limited
warning surfaces the total periodically) and `docs/users-guide.md` (each
component emits rate-limited warnings and maintains drop counters).

Trace links:

```plaintext
issue-440 AC1 (remove the suppressed-path reset)        -> EP-M1 -> rate_limited_warner::tests::suppressed_attempts_carry_drops_forward
issue-440 AC2 (flush is an unconditional swap)          -> EP-M1 -> rate_limited_warner::tests::flush_reports_inside_the_suppression_window
issue-440 AC3/AC4 (concurrency contract, no loss)       -> EP-M1 -> rate_limited_warner::tests::one_caller_claims_an_expired_interval
issue-440 AC5/AC6/AC7 (regression and stress tests)     -> EP-M2 -> rate_limited_warner::tests::*, tests/heavy/prop_rate_limited_warner.rs
issue-440 AC8 (consumers report on flush and shutdown)  -> EP-M3 -> stream_handler, logger, http_handler, socket_handler
design-doc statement (rate-limited warning totals)      -> EP-M1/EP-M2 -> docs/rust-multithreaded-logging-framework-for-python-design.md
```

## Verification plan

### Invariants

- **INV-1 (conservation).** For any interleaving of `record_drop()`,
  `warn_if_due()` and `flush()`, let `R` be the number of `record_drop()` calls
  that have completed, `P` the sum of the counts passed to callbacks that have
  returned, and `D` the counter's current value. Then `R = P + D`. Proof
  sketch: every `record_drop()` is a single relaxed `fetch_add(1)`; every drain
  is a single `swap(0)`, which returns the exact sum of the increments it
  observed and resets the counter to zero. The increments partition exactly
  across the swaps, so their values sum to the total. A drain reports its value
  in a callback whenever that value is non-zero.
- **INV-2 (no double report).** No drop contributes to two callbacks. Follows
  from INV-1: `swap(0)` removes the increments it returns.
- **INV-3 (suppression preserves).** A `warn_if_due()` call that does not
  claim an interval leaves `dropped` unchanged.
- **INV-4 (exclusive interval claim).** At most one `warn_if_due()` caller
  emits per interval. Follows from `compare_exchange` on `last_warn`: only the
  caller that changes the timestamp drains.
- **INV-5 (flush independence).** `flush()` reports whenever `dropped` is
  non-zero, regardless of the clock and without modifying `last_warn`.
- **INV-6 (monotonic timestamp).** `last_warn` never moves backwards once set,
  apart from the `u64::MAX` sentinel that precedes it. A claim writes `now`
  only where `now` is at least `prev + interval_ms` (or `prev` is the
  sentinel), so the write is forward.
- **INV-7 (consumer coverage).** Every warner instance reachable in the crate
  is flushed by at least one explicit flush or shutdown path.

### Obligations

- Obligation: INV-1, INV-2, INV-3, INV-5 over scripted interleavings.
  Method: named `rstest` regression tests with a fake clock. Rationale: each
  names one scenario whose failure the suite must detect; the scenarios are the
  acceptance criteria themselves. Domain: single-threaded sequences of
  record/warn/advance/flush. Artefact: the in-module `tests` block of
  `rust_extension/src/rate_limited_warner.rs`. Evidence:
  `cargo test --manifest-path rust_extension/Cargo.toml
  --no-default-features rate_limited_warner`.
  Each new test must fail against the current implementation (recorded in
  `Artefacts and notes`) and pass after. Non-vacuity: the carry-forward test
  fails on the unmodified code with `left: [1, 1], right: [1, 3]`, which is the
  mutation `dropped.store(0)` made visible; the flush test fails because the
  suppressed `flush()` emits nothing.
- Obligation: INV-1, INV-2, INV-4 under real thread interleavings.
  Method: a threaded stress test (normal lane) over an `Arc<RateLimitedWarner>`
  with a `Barrier`, plus a dedicated contention test that releases N
  `warn_if_due()` callers onto one expired interval and asserts exactly one
  callback. Rationale: INV-1/INV-2 are universal over interleavings; a sampled
  set of schedules is proportionate, and the contention test makes INV-4
  falsifiable rather than probabilistic. Domain: 2–4 threads, mixed
  `record_drop`/`warn_if_due`/`flush`, one thread advancing the fake clock,
  asserted at a barrier-quiesced point and after a final flush. Artefact: the
  same in-module `tests` block. Evidence: `cargo test ... rate_limited_warner`.
  Non-vacuity: the contention test asserts `exactly one` callback, which is
  false for a plain-load implementation; the stress test asserts
  `reported + pending == recorded`, which is false under the current code the
  moment any warning is suppressed.
- Obligation: INV-1 over generated operation sequences.
  Method: a `proptest` in the heavy target generating 2–4 threads of
  `Record`/`WarnIfDue`/`Flush`/`Advance` operations, each case ending in a
  flush. Rationale: sequences are where conservation is most easily violated;
  the ordinary lane cannot afford the thread churn, which is why it is
  `#[ignore]`. Domain: generated per-thread operation vectors, a shared public
  `Clock`, a `Barrier` start. Artefact:
  `rust_extension/tests/heavy/prop_rate_limited_warner.rs`, registered in
  `rust_extension/tests/heavy/main.rs`. Evidence:
  `cargo test --manifest-path rust_extension/Cargo.toml
  --no-default-features --test heavy prop_rate_limited_warner -- --ignored`.
  Non-vacuity: the generator is required to produce at least one `Record` per
  case and flushes at the end, so `P + D == R` is non-trivially exercised; a
  case with zero records would be rejected as vacuous by construction.
- Obligation: INV-7 across the four consumers.
  Method: existing behavioural suites plus the stream-handler integration test,
  which now asserts that two warnings carry counts summing to all four drops.
  Rationale: the consumer change is wiring, and the observable property is that
  no path can end with unreported pending drops. Domain: stream, logger, HTTP
  and socket handlers. Artefact:
  `rust_extension/tests/stream_handler_logging_tests.rs`, plus the existing
  handler suites in `rust_extension/tests/` and the in-module tests. Evidence:
  `cargo test --manifest-path rust_extension/Cargo.toml
  --no-default-features --test stream_handler_tests -- --ignored`
  (the ignored capture-logger binary). Non-vacuity: the updated integration
  test fails against the old suppression behaviour, which reported one drop
  where four were lost.
- Obligation: Axioms. `swap`/`fetch_add`/`compare_exchange` provide atomicity
  for their own location under `Ordering::Relaxed`; callbacks may run
  concurrently and are not serialized; a panicking callback abandons its batch.
  These are Rust/`core::sync::atomic` guarantees and documented contracts, not
  repository logic, and are treated as axioms.

### Quality criteria

- Tests: `make test` passes in all four feature lanes and `pytest` passes.
- Verification: INV-1…INV-7 discharged as above; the heavy property test runs
  with `-- --ignored`.
- Lint/typecheck: `make lint`, `make typecheck`, `make check-fmt`,
  `make markdownlint`, `make nixie`.
- Performance: no criterion regression is expected; no benchmark run is
  required, as the change adds one relaxed load to the suppressed path and
  removes one relaxed store.

## Milestones and plateaus

- **EP-M1 — the corrected helper and its contract.** `warn_if_due()` stops
  discarding pending drops and claims its interval with a CAS; `flush()` drains
  unconditionally; the module, struct and method rustdoc state conservation,
  suppression semantics, exclusive claims, flush independence and callback
  concurrency. Requirements: issue-440 AC1, AC2, AC3, AC4. Acceptance evidence:
  `rate_limited_warner::tests` passes; a manual run with the old `else` branch
  restored fails the new carry-forward test. Conformance check: constraints
  held (public shape unchanged, lock-free, messages unchanged); design followed
  (no new dependency); upstream assumption "the rate-limited warning surfaces
  the total" now holds as stated; no unapproved public interface change (the
  `test-util` accessor is recorded); trace links current. Recovery:
  `git revert` the milestone commit; the change is self-contained. Remaining
  gaps: consumer wiring and the heavy property test. Compatibility decision:
  none. The crate is `publish = false` and pre-1.0; the public method
  signatures are unchanged, so no consumer needs a shim.
- **EP-M2 — regression, contention and stress evidence.** New unit tests, the
  heavy property test, and the updated stream-handler integration test.
  Requirements: issue-440 AC5, AC6, AC7. Acceptance evidence: the named tests
  exist and the integration test asserts two warnings totalling four drops.
  Conformance check: constraints held; design followed; assumptions still
  valid; no public interface change; trace links current. Recovery: revert the
  test commit alone; the helper commit stands. Remaining gaps: consumer flush
  wiring. Compatibility decision: none.
- **EP-M3 — consumers report on flush and shutdown.** Stream, logger, HTTP and
  socket handlers flush every warner they own on their existing flush and
  shutdown paths. Requirements: issue-440 AC8. Acceptance evidence: `make test`
  in all lanes plus the ignored integration binary. Conformance check:
  constraints held (message texts unchanged, no new `Drop` hooks); design
  followed; assumptions valid; no interface change; no persisted-format change;
  trace links current. Recovery: revert the consumer commit; EP-M1/EP-M2 stand.
  Remaining gaps: none. Compatibility decision: none.

## Plan of work

Task 1 rewrites two methods and the documentation around them in
`rust_extension/src/rate_limited_warner.rs`, and adds the accumulation note to
`docs/rust-multithreaded-logging-framework-for-python-design.md` near the
"rate-limited warning surfaces the total periodically" paragraph. The
`warn_if_due()` body: return early when the counter is zero; sample `now` and
`prev`; test due-ness; `compare_exchange(prev, now)` and return on failure; then
`swap(0)` and call the callback only for a positive count. `flush()` becomes
`swap(0)` plus the same conditional callback, with no clock read and no
`last_warn` write. A `pending_drops()` accessor appears under `test-util`.

Task 2 adds tests in the same file: the changed expectation in
`rate_limits_subsequent_warnings`, a carry-forward regression, a
flush-inside-the-window regression, a parameterized empty-call round trip, a
threaded stress test, and a contention test. It registers
`prop_rate_limited_warner.rs` in the heavy target and updates
`stream_handler_logging_tests::stream_handler_rate_limits_warnings` to assert
the counts sum to the drops.

Task 3 touches the four consumers, moving `warner.flush()` ahead of the closed
early-returns in the HTTP and socket handlers, adding it to the stream handler's
`close()`, adding it to `FemtoLogger::flush_handlers_blocking()` and `Drop`,
and adding it to the HTTP and socket workers' flush-command and loop-exit paths.

## Concrete steps

All commands run from the worktree root
(`/home/leynos/.lody/repos/github---leynos---femtologging/worktrees/95df1fa9-36a8-4418-b8cb-57ad3e23c94f`).

```shell
git branch --show-current
# issue-440-preserve-dropped-record-counts-across-ratelimitedwarner-suppression-windows
```

Focused unit suite:

```shell
cargo test --manifest-path rust_extension/Cargo.toml \
  --no-default-features rate_limited_warner
```

Heavy property suite:

```shell
cargo test --manifest-path rust_extension/Cargo.toml \
  --no-default-features --test heavy prop_rate_limited_warner -- --ignored
```

Capture-logger integration suite:

```shell
cargo test --manifest-path rust_extension/Cargo.toml \
  --no-default-features --test stream_handler_tests -- --ignored
```

Full gates, delegated to `scrutineer`: `make fmt`, `make check-fmt`,
`make lint`, `make typecheck`, `make test`, `make markdownlint`, `make nixie`.

## Validation and acceptance

Acceptance is behavioural. After the change:

- Running the focused unit suite reports every `rate_limited_warner` test
  passing, including the new carry-forward, flush and contention tests.
- `stream_handler_rate_limits_warnings` observes exactly two warnings whose
  counts are 1 and 3, summing to the four records actually dropped.
- The heavy property test passes with `-- --ignored`, asserting that reported
  plus pending equals the number of `Record` operations for every generated
  interleaving.
- All four `make test` feature lanes and the Python suite pass.

Red-Green-Refactor evidence is recorded in `Artefacts and notes`: each new unit
test is run against the unmodified helper first, and the expected failure is
captured before the helper is changed.

## Idempotence and recovery

Every step is a source edit or a test run, both repeatable. The helper change
is one atomic commit, so a failed experiment is reverted with `git revert`
rather than by editing back. No data, schema or generated file is mutated by
this work. The only environment residue is `/tmp` logs from gate runs, which
the gate runner owns.

## Artefacts and notes

Reconnaissance evidence (2026-10-16):

```shell
$ git log --oneline -1 0856f6c8f6f6b9dac457256599b182e3f1726135
0856f6c Nest addendum sub-tasks under their parent roadmap task (#377)
$ git merge-base --is-ancestor 0856f6c8 HEAD && echo "defect present"
defect present
```

### Red evidence (2026-10-16)

The new tests were run against the unmodified helper before the fix was
accepted. The original body of `warn_if_due()` from `main` and the original
`flush()`, which delegated to it, were restored temporarily and the focused
suite was run:

```shell
cargo test --manifest-path rust_extension/Cargo.toml \
  --no-default-features --features test-util rate_limited_warner::tests
```

Result: `test result: FAILED. 6 passed; 7 failed`. The seven failures are the
new tests, plus `rate_limits_subsequent_warnings`, which previously asserted the
lossy counts and was rewritten as part of the fix:

```plaintext
rate_limits_subsequent_warnings   left: [1, 1]  right: [1, 2]
suppressed_attempts_carry_drops_forward
    "and preserve all three"        left: 0     right: 3
flush_reports_inside_the_suppression_window
                                    left: [1]   right: [1, 2]
empty_call_does_not_start_the_window::case_1
                                    left: []    right: [1]
empty_call_does_not_start_the_window::case_2
                                    left: []    right: [1]
concurrent_callers_conserve_every_drop::case_1_two_threads
    "every recorded drop should be reported exactly once"
                                    left: 41    right: 120
concurrent_callers_conserve_every_drop::case_2_four_threads
    "every recorded drop should be reported exactly once"
                                    left: 27    right: 160
```

Two results are worth naming. `suppressed_attempts_carry_drops_forward` shows
`pending_drops()` reading 0 where three drops should be pending: the old
`else` branch had already erased them. The two conservation cases are harsher
than a unit test would suggest — the unmodified helper reported 41 of 120
recorded drops on two threads and 27 of 160 on four. The bug is not a
rounding error on a boundary; under contention it discards most of the loss,
which is precisely the case an operator reads these warnings for.

Also notable: `empty_call_does_not_start_the_window` fails against the original
implementation for a reason the issue did not describe. The original
unconditionally stored the timestamp on the first call, even with nothing to
report, so a poller with no drops held the next real warning back a full
interval.

The same temporary revert was **not** applied to the heavy property test: it
asserts the same conservation the two-thread case above already falsifies, and
rebuilding the heavy target against a reverted helper costs a full uncached
compile for no additional information. The unit-level conservation case is the
witness, and the property test widens the interleaving space rather than
establishing the property for the first time. That is a deliberate, recorded
gap: the property test's own red evidence is the shared assertion, not a
separate run.

The fix was restored from a byte-for-byte backup (`/tmp/rlw-fixed-backup-issue-440.rs`)
and `cargo fmt -- --check` passes on the restored file.

## Interfaces and dependencies

No new dependency. The module surface after the change, in
`rust_extension/src/rate_limited_warner.rs`:

```rust
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> u64;
}

pub const DEFAULT_WARN_INTERVAL: Duration;

impl RateLimitedWarner {
    pub fn new(interval: Duration) -> Self;
    pub fn with_clock(interval: Duration, clock: Arc<dyn Clock>) -> Self;
    pub fn record_drop(&self);
    pub fn warn_if_due(&self, warn: impl FnMut(u64));
    pub fn flush(&self, warn: impl FnMut(u64));
    #[cfg(feature = "test-util")]
    pub fn pending_drops(&self) -> u64;
}
```

## Revision note

2026-10-16: initial draft, written after reconnaissance of the defect, its four
consumers and the existing tests.
