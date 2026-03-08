# Complete timed rotating handler coverage

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item `2.1.2` is still open in [docs/roadmap.md](../roadmap.md): add
`FemtoTimedRotatingFileHandler` so femtologging covers both size-based and
time-based rotating file handlers. This matters because the project is first a
Python logging library implemented in Rust, and Python users reasonably expect
stdlib-style timed file rotation to exist wherever size-based rotation already
exists.

Observable success after implementation:

1. Python code can construct and use a timed rotating file handler through the
   public package surface and through `dictConfig`.
2. Rotation occurs on the handler worker thread, not on the producer path, and
   queued logging remains non-blocking for callers.
3. Rust unit tests using `rstest`, Python behavioural tests using
   `pytest-bdd`, and syrupy snapshot tests all cover happy paths, unhappy
   paths, and edge cases without relying on flaky wall-clock sleeps.
4. The design and configuration documents describe the final behaviour, and
   roadmap item `2.1.2` is marked done only after all quality gates are green.

## Context and orientation

The current rotating implementation is size-based only:

- `rust_extension/src/handlers/rotating/` contains the existing
  `FemtoRotatingFileHandler` split into `core.rs`, `python.rs`, and
  `strategy.rs`.
- `rust_extension/src/handlers/rotating_builder.rs` and
  `rust_extension/src/handlers/rotating_builder/python_bindings.rs` provide the
  builder and PyO3 wrappers for the size-based handler.
- `femtologging/config.py` maps stdlib handler class names such as
  `logging.handlers.RotatingFileHandler` to the builder layer used by
  `dictConfig`.
- `tests/test_rotating_handler.py`,
  `tests/steps/test_rotating_rotation_steps.py`, and
  `tests/features/rotating_handler_rotation.feature` show the established
  Python testing pattern for direct handler construction, BDD scenarios, and
  syrupy snapshots.

There is no timed rotating handler, timed rotating builder, timed handler class
mapping, or timed rotating test suite in the current tree.

The design sources that constrain this plan are:

- [docs/rust-multithreaded-logging-framework-for-python-design.md](../rust-multithreaded-logging-framework-for-python-design.md)
  section `3.4`, which lists `TimedRotatingFileHandler` parity as a core
  handler.
- [docs/rust-multithreaded-logging-framework-for-python-design.md](../rust-multithreaded-logging-framework-for-python-design.md)
  section `6.3.1`, which requires rotation logic to stay in the worker thread.
- [docs/configuration-design.md](../configuration-design.md), which makes the
  builder API the canonical configuration surface and keeps Python bindings as
  thin wrappers over Rust builders.

## Constraints

- Keep timed rotation on the consumer thread. Producer-side log calls must not
  perform filesystem rollover or time-schedule work beyond queueing records.
- Preserve existing size-based rotation behaviour and public APIs. Adding timed
  rotation must not regress `FemtoRotatingFileHandler`,
  `RotatingFileHandlerBuilder`, `HandlerOptions`, or their tests.
- Treat this as a Python-first feature. The final implementation must include
  Rust core logic, Python bindings, package exports, type stubs, and
  `dictConfig`/builder integration in the same milestone.
- Do not rely on wall-clock sleeps for core correctness tests. The
  implementation must provide a deterministic way to test time-based rollover.
- Keep files under 400 lines by following the existing split-by-concern module
  structure used for the size-based rotating handler.
- Public PyO3 APIs must return structured Python errors (`PyResult<T>` or
  `From<DomainError> for PyErr`) rather than panicking or exposing opaque Rust
  errors.
- Comments and documentation must use en-GB-oxendict spelling.
- Before the feature is considered complete, all of the following must pass:
  `make check-fmt`, `make typecheck`, `make lint`, `make test`,
  `make markdownlint`, and `make nixie`.

## Tolerances (exception triggers)

- Scope: if implementation grows beyond 20 touched files or roughly 1,200 net
  lines before tests are added, stop and reassess whether the work should be
  split into a handler-core milestone and a configuration-surface milestone.
- Interface: if implementing timed rotation requires changing an existing
  public signature instead of adding new types, stop and escalate before
  breaking compatibility.
- Dependencies: if a new crate is required for time/date calculations, allow at
  most one well-scoped dependency and record the rationale in the design doc.
  If more than one new dependency is needed, stop and escalate.
- Semantics: if CPython-compatible handling of `when`, `utc`, or `at_time`
  conflicts with the current worker-thread design, stop and document the
  competing options in `Decision Log` before proceeding.
- Testing: if deterministic tests cannot be achieved without real-time sleeps
  longer than 250 ms, stop and add a fake-clock or injected-scheduler path
  instead of accepting flakiness.
- Iterations: if the same failing gate has been retried three times without a
  clear new hypothesis, stop and document the blocker before more churn.

## Risks

- Risk: calendar-based schedules (`MIDNIGHT`, weekday rotation, and
  `at_time`) are easy to implement incorrectly around UTC/local-time
  boundaries. Severity: high Likelihood: medium Mitigation: isolate
  next-rollover calculation in a pure schedule component with parameterised
  `rstest` cases before wiring the handler.

- Risk: timed rotation tests become flaky if they depend on real time passing.
  Severity: high Likelihood: high Mitigation: add an injected clock or
  test-only scheduler control and keep BDD tests deterministic.

- Risk: adding timed rotation only to Rust core but not to builders,
  `dictConfig`, exports, and stubs would leave Python users with an incomplete
  feature. Severity: high Likelihood: medium Mitigation: treat Python surface
  integration as part of the definition of done, not follow-up work.

- Risk: retention semantics for timed rotation differ from size-based rotation,
  especially when `backup_count == 0`. Severity: medium Likelihood: medium
  Mitigation: document the chosen semantics explicitly in the design doc and
  test both pruning and non-pruning cases.

- Risk: worker-thread rotation may need test hooks similar to the existing
  fresh-file failure hook, and poorly scoped hooks could leak into the public
  API. Severity: medium Likelihood: medium Mitigation: keep any fake-clock or
  failure-injection helpers private or clearly test-only, mirroring the current
  rotating-handler pattern.

## Progress

- [x] 2026-03-08: Reviewed roadmap, configuration roadmap note, design docs,
  current rotating handler code, configuration mapping, and existing rotating
  test patterns.
- [x] 2026-03-08: Wrote this draft ExecPlan in
  `docs/execplans/2-1-2-complete-rotating-handler-coverage.md`.
- [ ] Add a pure timed-rotation schedule component with deterministic tests.
- [ ] Implement the Rust timed rotating handler core and worker-thread rollover
  path.
- [ ] Add the timed rotating builder and Python bindings.
- [ ] Wire the handler into package exports, stubs, and `dictConfig`.
- [ ] Add Rust `rstest` coverage, Python `pytest` unit coverage, BDD scenarios,
  and syrupy snapshots.
- [ ] Update design/configuration docs and mark roadmap item `2.1.2` done.
- [ ] Run and pass all quality gates.

## Surprises & Discoveries

- `docs/configuration-roadmap.md` no longer contains separate execution detail;
  it now redirects to `docs/roadmap.md`, so the consolidated roadmap is the
  source of truth.
- The current working tree does not contain any prior
  `2-1-2-complete-rotating-handler-coverage.md` file, even though project
  memory references an earlier draft. This plan is therefore authored from the
  current tree, not refreshed from an existing file.
- `femtologging/config.py` hardcodes supported handler class names. Timed
  rotation will not be reachable from `dictConfig` unless that mapping is
  updated.
- The existing size-based rotating handler already shows the preferred shape
  for this work: split core logic from Python bindings and keep behaviour tests
  deterministic through test support hooks rather than sleeps.

## Decision Log

- Decision: implement timed rotation as a separate handler family rather than
  overloading the existing size-based rotating handler types. Rationale:
  size-based and time-based rotation have different configuration models,
  validation rules, and retention semantics. A dedicated
  `FemtoTimedRotatingFileHandler` and `TimedRotatingFileHandlerBuilder` keep
  the public API precise and avoid contaminating `HandlerOptions` with
  unrelated fields.

- Decision: include builder, direct Python handler construction, package
  exports, stubs, and `dictConfig` wiring in the same milestone. Rationale: the
  project is Python-first, and roadmap item `2.1.2` would not be meaningfully
  complete if only the Rust core type existed.

- Decision: add deterministic time control for tests before writing broad
  behavioural coverage. Rationale: waiting for real clock boundaries would make
  both Rust and Python tests flaky and slow, which is incompatible with the
  repository's testing standards.

- Decision: follow stdlib-style timed rotation semantics for the supported
  fields `when`, `interval`, `backupCount`, `utc`, and `atTime`, and record any
  intentionally unsupported values in the design doc. Rationale: the design
  document names these exact features, and Python users expect them from a
  `TimedRotatingFileHandler`.

- Decision: record retention semantics explicitly, including the timed-rotation
  meaning of `backup_count == 0`. Rationale: size-based rotation currently uses
  zero values to disable rotation, while timed rotation may still rotate but
  retain all timestamped files. This difference must be documented instead of
  left implicit.

## Plan of work

## Stage 1: Add a deterministic timed-rotation schedule core

Create a new timed-rotation module parallel to the size-based handler, under a
new directory such as `rust_extension/src/handlers/timed_rotating/`. Keep the
module split by concern from the start:

- `mod.rs` for exports and wiring.
- `core.rs` for the handler wrapper and worker integration.
- `schedule.rs` for next-rollover calculation and filename suffix rules.
- `python.rs` for PyO3-only bindings.
- optional `clock.rs` or equivalent for a production clock and a test clock.

The first deliverable is not the full handler. It is a pure schedule component
that can answer two questions deterministically:

1. Given the current instant and config, when is the next rollover?
2. Given the rollover instant, what suffix should the rotated filename use?

Support the stdlib-style `when` forms needed by the design doc:

- `"S"` for seconds
- `"M"` for minutes
- `"H"` for hours
- `"D"` for days
- `"MIDNIGHT"`
- `"W0"` through `"W6"` for weekday rotation

Validate that:

- `interval` must be greater than zero.
- `at_time` is only accepted where the schedule meaningfully uses a time of
  day.
- invalid `when` values fail fast with a targeted configuration error.
- UTC/local schedule selection is represented in config instead of being an
  ad-hoc boolean passed around the worker.

Before touching the worker path, add Rust tests in the new schedule module
using `rstest` to cover:

- hourly, daily, midnight, and weekday next-rollover calculations
- `utc=True` versus local-time behaviour
- `at_time` handling for midnight and weekday schedules
- filename suffix generation and lexicographic ordering
- invalid `when`, zero `interval`, and invalid `at_time` combinations

Observable checkpoint: a targeted Rust test run for the new schedule module
passes without creating a handler or sleeping.

## Stage 2: Implement the Rust timed rotating handler

Add `FemtoTimedRotatingFileHandler` as a file-backed worker-thread wrapper that
reuses the existing file-handler infrastructure in the same way
`FemtoRotatingFileHandler` does today.

The worker path must:

1. ask the injected schedule whether rollover is due before each write,
2. flush and rotate entirely on the worker thread when due,
3. reopen the base log file, and
4. prune or retain timestamped backups according to the chosen retention
   semantics.

Do not fold timed logic into the existing size-based `FileRotationStrategy`.
Timed rotation has different trigger logic and filename management, so it
should use a separate strategy type even if both share lower-level file
operations later.

The handler core should expose read-only inspection methods needed by tests and
Python wrappers, similar to the current `rotation_limits()` pattern. Expected
inspection data includes the resolved schedule fields and retention count.

Rust unit and behaviour coverage for this stage should prove:

- rollover occurs when the fake clock crosses the scheduled boundary
- no rollover occurs before the boundary
- timestamped backups are named and pruned correctly
- `backup_count == 0` follows the documented timed-rotation semantics
- shutdown, flush, and drop remain safe after rotation
- producer threads remain non-blocking while rotation work happens in the
  consumer thread

If the implementation needs fault-injection support for reopen failures or
clock control, follow the existing `fresh_failure` pattern used by the
size-based handler and keep the helper test-only.

Observable checkpoint: new Rust timed-handler tests fail before the handler
exists and pass once the worker-thread implementation is complete.

## Stage 3: Add builder and Python bindings

Introduce a new `TimedRotatingFileHandlerBuilder` in
`rust_extension/src/handlers/timed_rotating_builder.rs` with a sibling
`python_bindings.rs`, following the same split used by the size-based rotating
builder.

The builder should own:

- the file path
- shared queue settings from `FileLikeBuilderState`
- timed rotation config: `when`, `interval`, `backup_count`, `utc`, and
  `at_time`

Validation belongs in Rust builder code first, with PyO3 wrappers rejecting
type and range errors as early as possible.

Also add a direct Python handler wrapper class,
`FemtoTimedRotatingFileHandler`, under the timed rotating module. Give it a
construction surface that matches the repository's current pattern for direct
handlers while still exposing the timed fields clearly. If a dedicated options
object is used for direct construction, define and export it alongside the
handler and document the final naming in the design doc.

Files that should be updated in this stage include:

- `rust_extension/src/handlers/mod.rs`
- `rust_extension/src/lib.rs`
- `rust_extension/src/python_module.rs`
- `femtologging/__init__.py`
- `femtologging/_femtologging_rs.pyi`

Update Rust/Python registration tests so the new builder and handler appear in
the module bindings alongside the existing rotating types.

Observable checkpoint: Python can import the timed handler and timed builder,
and stub/typecheck coverage sees the new names.

## Stage 4: Wire timed rotation into configuration flows

Extend configuration parsing so timed rotation is reachable from the canonical
Python configuration surfaces:

- add the timed builder variant to `rust_extension/src/config/types.rs`
- update `rust_extension/src/config/py.rs` so `HandlerBuilder` can extract the
  timed builder from Python objects
- extend `femtologging/config.py` so `_HANDLER_CLASS_MAP` recognises stdlib and
  femtologging timed rotating handler class names

At minimum, support these class aliases:

- `logging.handlers.TimedRotatingFileHandler`
- `logging.TimedRotatingFileHandler`
- `femtologging.TimedRotatingFileHandler`
- `femtologging.FemtoTimedRotatingFileHandler`

Add or update `dictConfig` tests so a timed rotating handler can be built from
configuration and unsupported keys fail with explicit messages.

Observable checkpoint: `dictConfig` can construct a timed rotating handler from
a handler class string and valid kwargs.

## Stage 5: Add comprehensive tests across Rust and Python

Rust coverage should use `rstest` and stay close to the schedule and worker
components so failures point at a single concern. Prefer parameterised cases
over repeated setup code.

Python coverage must include all three layers already expected in this
repository:

1. `pytest` unit tests for constructor and builder validation.
2. `pytest-bdd` scenarios for behavioural rollover flows.
3. syrupy snapshot assertions for resulting file layouts and configuration
   dictionaries.

Add new or updated Python tests in the style of the existing rotating tests:

- `tests/test_timed_rotating_handler.py` for direct-construction validation
- `tests/features/handler_builders.feature` and
  `tests/steps/test_handler_builders_steps.py` for timed builder scenarios and
  snapshots
- `tests/features/timed_rotating_handler.feature` and a new steps file for BDD
  rollover behaviour
- `tests/steps/__snapshots__/...` snapshot files for both builder dictionaries
  and rotated-file layouts
- `tests/test_dict_config.py` or the existing dictConfig BDD/unit suites for
  configuration-path coverage

Required Python scenarios:

- happy paths for interval-based rollover, midnight rollover, and weekday or
  `at_time` rollover
- no-rollover-before-boundary cases
- invalid `when`, zero `interval`, invalid `at_time`, and invalid builder
  field types
- retention pruning and the documented `backup_count == 0` behaviour
- `utc=True` behaviour distinct from local scheduling

Prefer fake-clock control in BDD steps over `time.sleep()`. If a Python test
must poll for worker completion, use the existing repo helper style with tight
timeouts and explicit comments explaining why polling remains deterministic.

Observable checkpoint: targeted Python timed-rotation tests pass reliably on
repeat runs.

## Stage 6: Update design documents and close the roadmap item

After the code and tests are green, update the docs so the written design
matches the shipped behaviour:

- `docs/rust-multithreaded-logging-framework-for-python-design.md`
- `docs/configuration-design.md`
- `docs/formatters-and-handlers-rust-port.md` if the handler overview needs
  refreshed implementation detail
- `docs/roadmap.md`

Record at least these decisions in the design document:

- supported `when` values and any intentional exclusions
- how `utc` and `at_time` are interpreted
- worker-thread rollover flow for timed rotation
- retention semantics, especially for `backup_count == 0`
- how deterministic testing is achieved without flaky wall-clock waits

Only once all tests and documentation updates are complete should roadmap item
`2.1.2` be marked `[x]`.

## Validation and evidence

Run the full repository gates with `tee` so failures can be inspected after the
fact:

```bash
set -o pipefail && make check-fmt | tee /tmp/2-1-2-check-fmt.log
set -o pipefail && make typecheck | tee /tmp/2-1-2-typecheck.log
set -o pipefail && make lint | tee /tmp/2-1-2-lint.log
set -o pipefail && make test | tee /tmp/2-1-2-test.log
set -o pipefail && make markdownlint | tee /tmp/2-1-2-markdownlint.log
set -o pipefail && make nixie | tee /tmp/2-1-2-nixie.log
```

The implementation is complete only when:

- the timed rotating Rust and Python tests are present and green,
- the full gate suite above succeeds,
- the design/configuration docs describe the final behaviour, and
- [docs/roadmap.md](../roadmap.md) marks `2.1.2` as done.

## Approval gate

This document is the draft phase required by the `execplans` workflow. Do not
begin implementation until the user explicitly approves the plan or requests
specific revisions to it.

## Outcomes & Retrospective

This section remains intentionally incomplete until implementation finishes.
When the work is done, replace this note with:

- what shipped,
- which risks materialised,
- what changed from the draft,
- exact gate results, and
- any lessons worth carrying into later handler work.
