# Deliver a tracing-subscriber layer for Rust ecosystem integration

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item `3.3.3` in [docs/roadmap.md](../roadmap.md) will be complete when
Rust code using `tracing` can emit spans and events that flow through
femtologging's existing handlers in the same process, especially in mixed
Python/Rust extension deployments. This matters because femtologging is first
and foremost a Python logging library implemented in Rust: the tracing bridge
must strengthen that Python-facing story rather than creating a Rust-only side
channel.

Observable success means all of the following are true:

1. A Rust caller can install a `tracing_subscriber::Layer` supplied by
   femtologging and see `tracing::event!` output arrive in femtologging
   handlers.
2. Logger routing, level filtering, and flush semantics remain consistent with
   the existing `log::Log` bridge in `rust_extension/src/log_compat.rs`.
3. Structured event fields and selected span context appear in the
   `FemtoLogRecord` metadata that Python `handle_record` handlers already
   receive.
4. Rust `rstest` unit coverage, Python behavioural tests with `pytest-bdd`,
   and syrupy snapshots cover happy paths, unhappy paths, and edge cases.
5. The design and user documentation explain the final behaviour, and roadmap
   item `3.3.3` is marked done only after all repository gates are green.

## Context and orientation

The current tree already contains the first Rust ecosystem integration step:
`rust_extension/src/log_compat.rs` implements `FemtoLogAdapter`, installs a
global `log::Log`, normalizes Rust `target`s to femtologging logger names, and
dispatches converted `FemtoLogRecord`s through the normal handler queues.

The key files and modules that define the starting point are:

- `rust_extension/src/log_compat.rs`, the existing pattern for Rust-side log
  facade integration.
- `rust_extension/src/log_record.rs`, which defines `FemtoLogRecord` and
  `RecordMetadata`, including the `key_values` map that can carry structured
  tracing fields.
- `rust_extension/src/logger/mod.rs`, whose `log_with_metadata` and
  `dispatch_record` paths already enforce logger-level checks and route records
  to handlers.
- `rust_extension/src/python_module.rs`,
  `femtologging/_rust_compat.py`, and `femtologging/_femtologging_rs.pyi`,
  which show how feature-gated Rust integration helpers are surfaced to Python
  users today.
- `tests/features/rust_log_compat.feature`,
  `tests/steps/test_rust_log_compat_steps.py`, and
  `tests/steps/__snapshots__/test_rust_log_compat_steps.ambr`, which provide
  the existing behavioural and snapshot pattern for Rust ecosystem bridges.

The design sources that constrain this plan are:

- [docs/rust-multithreaded-logging-framework-for-python-design.md](../rust-multithreaded-logging-framework-for-python-design.md)
  section `6.4`, which requires interoperability with the Rust logging
  ecosystem.
- [docs/adr-002-journald-and-otel-support.md](../adr-002-journald-and-otel-support.md)
  section `Phase 2`, which describes a `tracing_subscriber::Layer` that should
  capture `tracing` events, compose with OpenTelemetry layers, and avoid
  feedback loops.
- [docs/configuration-design.md](../configuration-design.md), which states that
  Rust ecosystem integration is part of the broader configuration and
  compatibility story rather than an unrelated Rust-only add-on.
- [docs/multithreading-in-pyo3.md](../multithreading-in-pyo3.md), which
  constrains how any Python interaction may occur from Rust threads in an
  extension module.
- [docs/rust-testing-with-rstest-fixtures.md](../rust-testing-with-rstest-fixtures.md)
  and [docs/rust-doctest-dry-guide.md](../rust-doctest-dry-guide.md), which
  define the repo's expectations for `rstest` fixtures and Rust documentation
  examples.

## Constraints

- Preserve the current femtologging hot path. The tracing layer must convert
  `tracing` events into `FemtoLogRecord`s and enqueue them through the existing
  logger/handler pipeline rather than inventing a second dispatch path.
- Keep the project Python-first. Even if the new public type is Rust-facing,
  the feature is only done when the Python package, compatibility shims,
  behavioural tests, and documentation explain how mixed Python/Rust users
  benefit from it.
- Do not require Python callers to use `tracing` directly. The new layer must
  be an additive Rust integration surface for Python extensions and hybrid
  applications, not a replacement for femtologging's Python API.
- Avoid panics and opaque FFI failures. Any Python-exposed helper functions
  added for tests or installation must return `PyResult<T>` and convert Rust
  errors to structured Python exceptions.
- Prevent logging feedback loops. Events emitted from femtologging's own
  internal diagnostics must not recursively re-enter the tracing layer.
- Preserve the current `log-compat` feature behaviour. Adding tracing support
  must not regress `setup_rust_logging()`, `_emit_rust_log()`, or the existing
  `log::Log` tests and docs.
- Keep code files under 400 lines by splitting new tracing-layer logic by
  concern, following the timed rotating and `log_compat` patterns already in
  the tree.
- Before the roadmap entry is marked done, all of the following must pass:
  `make check-fmt`, `make typecheck`, `make lint`, `make test`,
  `make markdownlint`, and `make nixie`.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 16 touched files or roughly
  900 net new lines before tests and docs are added, stop and reassess whether
  the work should be split into a minimal event-only layer and a later
  span-context enhancement.
- Interface: if supporting the tracing layer requires changing an existing
  public Python signature or breaking the `log-compat` API, stop and escalate
  before proceeding.
- Dependencies: allow at most two new Rust dependencies and only if they are
  directly required for `tracing` integration. If the work appears to require
  more than `tracing` and `tracing-subscriber`, stop and document why.
- Semantics: if span context cannot be attached without an invasive redesign of
  `FemtoLogRecord` or `logger::dispatch_record`, deliver event support first
  and escalate before broadening the payload model.
- Performance: if event conversion requires blocking on Python, holding the GIL
  for more than logger resolution, or allocating large temporary structures on
  every event, stop and redesign the conversion path before merging.
- Testing: if deterministic behavioural coverage cannot be achieved without
  subprocess coordination or sleeps longer than 250 ms, stop and add a smaller
  Rust-side test harness instead of accepting flakiness.
- Iterations: if the same failing gate is retried three times without a new
  hypothesis, stop and document the blocker in `Decision Log`.

## Risks

- Risk: `tracing` spans expose richer context than `FemtoLogRecord` can
  naturally carry today. Severity: high Likelihood: medium Mitigation: start
  with event support plus a bounded span-context subset stored in
  `metadata.key_values`, and document any intentionally deferred fields.

- Risk: incorrect field visitation may lose event data or stringify values
  inconsistently. Severity: high Likelihood: medium Mitigation: isolate field
  extraction in a small visitor module with targeted `rstest` cases covering
  scalars, booleans, debug-only values, and repeated keys.

- Risk: routing by `tracing` target may diverge from the existing `log::Log`
  bridge and surprise users. Severity: medium Likelihood: medium Mitigation:
  reuse the same target normalization and root-fallback rules as
  `log_compat.rs` wherever possible.

- Risk: femtologging's own diagnostics could recurse through the new layer.
  Severity: high Likelihood: medium Mitigation: define and test a concrete
  ignore rule for `femtologging` and `femtologging.log_compat` targets before
  wiring the layer into examples or helpers.

- Risk: feature gating may become confusing if tracing support is enabled by
  default without clear docs. Severity: medium Likelihood: medium Mitigation:
  choose an explicit Cargo feature, update Python compatibility shims, and add
  migration notes alongside the existing `log-compat` guidance.

- Risk: behavioural tests for mixed Rust/Python integration may need a fresh
  tracing subscriber per scenario, and global subscriber state is harder to
  reset than loggers. Severity: high Likelihood: high Mitigation: prefer a
  local `tracing_subscriber::Registry` in Rust unit tests and use subprocess
  isolation only for Python BDD scenarios that must validate fresh-process
  install semantics.

## Progress

- [x] 2026-03-27: Reviewed roadmap item `3.3.3`, design section `6.4`, ADR 002
  phase 2, and current configuration design notes.
- [x] 2026-03-27: Reviewed the existing `log::Log` bridge, Python compatibility
  surface, and BDD/snapshot coverage pattern.
- [x] 2026-03-27: Wrote this draft ExecPlan in
  `docs/execplans/3-3-3-tracing-subscriber-layer.md`.
- [ ] Add a feature-gated tracing integration module and dependency wiring.
- [ ] Implement event conversion, logger resolution, and feedback-loop guards.
- [ ] Decide and implement the minimal supported span-context propagation.
- [ ] Expose any required Python-facing compatibility helpers and type stubs.
- [ ] Add Rust `rstest` coverage, Python behavioural coverage, and syrupy
  snapshots.
- [ ] Update design and user documentation, then mark roadmap item `3.3.3`
  done.
- [ ] Run and pass all repository quality gates.

## Surprises & Discoveries

- The repository already has a complete `log::Log` bridge with both Rust unit
  tests and Python BDD snapshots. The tracing layer should extend that pattern
  rather than inventing a brand-new integration shape.
- `RecordMetadata.key_values` already exists and reaches Python
  `handle_record` callbacks, so the first tracing milestone does not need a new
  record type solely to carry structured event fields.
- The crate currently has no `tracing` or `tracing-subscriber` dependencies
  and no `tracing` feature in `rust_extension/Cargo.toml`, so feature design is
  part of the work, not follow-up cleanup.
- The current Python compatibility layer only exposes Rust integration helpers
  for `log-compat`. If the tracing layer needs Python-visible test hooks or
  installation wrappers, corresponding updates will be needed in
  `_rust_compat.py`, `__init__.py`, and the `.pyi` stub.
- The roadmap item is phrased narrowly as a `tracing_subscriber::Layer`, but
  ADR 002 also expects documentation about OpenTelemetry composition and
  feedback-loop boundaries. The code and docs must therefore land together.

## Decision Log

- Decision: treat the first implementation as an event bridge with bounded
  span-context enrichment, not full tracing semantic export. Rationale: both
  the design doc and ADR 002 explicitly frame events as the primary log-record
  analogue, while span handling may remain minimal at first.

- Decision: mirror `log_compat.rs` for logger resolution, level mapping, and
  root fallback unless a tracing-specific incompatibility is proven. Rationale:
  Python users expect Rust `log` and `tracing` records to land in the same
  femtologging hierarchy.

- Decision: prefer a dedicated Cargo feature such as `tracing-compat` rather
  than folding tracing support into `log-compat`. Rationale: `tracing`
  integration adds different dependencies and may be desirable independently of
  the global `log::Log` install path.

- Decision: keep any Python-facing additions minimal and test-oriented unless a
  real user workflow requires a Python wrapper. Rationale: the layer itself is
  a Rust subscriber component, but the project still needs Python behavioural
  validation and documentation because it ships as a Python package.

- Decision: document and test explicit loop-prevention rules instead of relying
  on vague "do not log internally" guidance. Rationale: recursive logging bugs
  are subtle and expensive to debug in multithreaded extension code.

## Plan of work

## Stage 1: Add feature gating and a minimal tracing module skeleton

Extend `rust_extension/Cargo.toml` with SemVer version requirements for
`tracing` and `tracing-subscriber` using Cargo's default implicit caret
behaviour, then add a feature such as `tracing-compat`. Keep the default
feature decision conservative: if enabling the layer by default risks
unexpected dependency or compile-time expansion for Python-only users, leave it
opt-in and document that choice.

Create a new module, likely `rust_extension/src/tracing_compat.rs`, parallel to
`log_compat.rs`. The module should begin with a `//!` comment and own the
following responsibilities:

- mapping `tracing::Level` to `FemtoLevel`
- normalizing `tracing` targets to femtologging logger names
- converting event metadata and fields into `FemtoLogRecord`
- enforcing ignore rules that prevent femtologging's own diagnostics from
  looping back through the layer
- exposing a small public Rust API for constructing the layer

If the file approaches 400 lines, split it into a directory module such as
`rust_extension/src/tracing_compat/` with `mod.rs`, `layer.rs`, `visitor.rs`,
and `tests.rs`.

Observable checkpoint:
`cargo check --manifest-path rust_extension/Cargo.toml --features tracing-compat`
 succeeds before any Python-facing work begins.

## Stage 2: Implement event conversion and logger dispatch

Implement a concrete layer type such as `FemtoTracingLayer` that satisfies
`tracing_subscriber::Layer<S>`. Start with `on_event`, because ADR 002 treats
events as the log-record analogue. The conversion rules should be explicit:

1. Resolve the logger name from `event.metadata().target()`, reusing the same
   `::` to `.` normalization and root fallback semantics as `log_compat.rs`.
2. Map the `tracing` level to `FemtoLevel`.
3. Build `RecordMetadata` from file, line, module path, current thread data,
   and extracted event fields.
4. Choose the message text predictably. If the event contains a `message`
   field, use that string as the primary message and keep other fields in
   `key_values`. If there is no `message` field, fall back to a stable textual
   representation documented in the design doc.
5. Dispatch through the resolved `FemtoLogger` using existing queue-based
   methods, not synchronous handler calls.

Add a field visitor that converts supported tracing values into strings stored
in `metadata.key_values`. At minimum cover strings, integers, floats, booleans,
and debug-rendered fallback values. If duplicate keys appear, define whether
the last write wins or whether duplicates are rejected, and record that choice
in the design doc.

Observable checkpoint: a Rust unit test can attach a `CollectingHandler`,
invoke the layer with a synthetic `tracing::Event`, and assert that the
captured `FemtoLogRecord` contains the expected logger, level, message, source
location, and structured fields.

## Stage 3: Add bounded span-context support

Implement the smallest useful span story that meets the roadmap item without
pretending to provide full tracing semantics. The recommended scope is:

- in `on_new_span` and `on_record`, cache selected span fields in the span's
  extensions
- in `on_event`, merge the currently active span fields into
  `metadata.key_values` using a reserved prefix such as `span.` or
  `tracing.span.`
- in `on_enter` / `on_exit`, avoid heavy bookkeeping unless required for the
  chosen merge model

Do not attempt distributed trace propagation, native OpenTelemetry export, or
full span lifecycle mirroring in this milestone. If those become necessary to
make the layer useful, stop and escalate because that exceeds roadmap item
`3.3.3`.

Observable checkpoint: a Rust unit test using `tracing::info_span!` proves that
an event emitted inside a span includes the chosen span fields in
`metadata.key_values`, while an event emitted outside any span does not.

## Stage 4: Expose the integration surface cleanly

Decide the public Rust API shape and document it in both Rustdoc and the user
guide. A likely shape is one or both of:

- a constructor such as `femtologging_rs::tracing_compat::layer()`
- a concrete exported type `FemtoTracingLayer`

Keep the API explicit rather than installing a global tracing subscriber inside
femtologging. Unlike the `log` facade, `tracing` is normally composed by
building a subscriber registry, and the docs should respect that ecosystem
pattern.

If Python behavioural tests need Rust helpers to emit tracing events or to
build a test subscriber in a subprocess, add narrowly scoped feature-gated
`#[pyfunction]` helpers in the same style as `_emit_rust_log()`. Update all of
the following in the same change:

- `rust_extension/src/python_module.rs`
- `femtologging/_rust_compat.py`
- `femtologging/__init__.py`
- `femtologging/_femtologging_rs.pyi`

If no Python-visible helper is needed, record that decision in `Decision Log`
and keep the Python package surface unchanged.

Observable checkpoint: Rustdoc examples compile in dry-run mode, and any
Python-visible compatibility helpers fail with a clear error when the extension
is built without the tracing feature.

## Stage 5: Add Rust unit tests with rstest

Create focused Rust tests beside the new tracing module, using `rstest`
fixtures rather than ad-hoc repeated setup. At minimum cover:

- level mapping from `tracing::Level` to `FemtoLevel`
- target normalization from Rust module paths to femtologging logger names
- root fallback when the target is invalid
- event field extraction for strings, numbers, booleans, and debug fallback
- message extraction when `message` is present and when it is absent
- logger threshold behaviour
- loop-prevention filters for femtologging-owned targets
- span-context merge behaviour for active and inactive spans
- flush bridging, if the public API exposes a flush-related helper or if the
  layer shares machinery with `manager::flush_all_handlers`

Use `crate::test_utils::collecting_handler::CollectingHandler` so assertions
are made on real `FemtoLogRecord`s rather than on formatted strings.

## Stage 6: Add Python behavioural tests and syrupy snapshots

Create a new feature file and step module parallel to the existing
`rust_log_compat` suite, for example:

- `tests/features/rust_tracing_compat.feature`
- `tests/steps/test_rust_tracing_compat_steps.py`
- `tests/steps/__snapshots__/test_rust_tracing_compat_steps.ambr`

The behavioural scenarios should focus on Python-observable behaviour in mixed
Python/Rust usage, not on re-testing every Rust unit detail. Cover at least:

1. A tracing event emitted from Rust reaches a femtologging stream handler and
   matches a stored snapshot.
2. Logger-level filtering still suppresses low-level tracing events.
3. Structured tracing fields arrive in Python `handle_record` payloads.
4. Span context is present on nested events in the chosen key format.
5. A loop-prevention or unsupported-install scenario fails in a controlled way
   and matches a snapshot.

Follow the existing `log-compat` pattern for module-level feature detection and
skip behaviour cleanly when the extension lacks the required Cargo feature. Use
subprocess isolation only when a scenario must validate fresh-process
subscriber installation semantics.

## Stage 7: Update documentation and roadmap state

Update the design and user-facing docs together so the implemented behaviour is
discoverable and consistent:

- expand
  [docs/rust-multithreaded-logging-framework-for-python-design.md](../rust-multithreaded-logging-framework-for-python-design.md)
   section `6.4` with the final layer shape, supported field capture, span
  behaviour, and loop-prevention rules
- update [docs/configuration-design.md](../configuration-design.md) section `4`
  so the Rust ecosystem integration description matches the actual feature and
  installation API
- update [docs/rust-extension.md](../rust-extension.md) with a new section next
  to the existing Rust `log` crate bridge, showing how to compose
  `FemtoTracingLayer` with `tracing_subscriber::Registry` and, if appropriate,
  OpenTelemetry layers
- update [docs/contents.md](../contents.md) if a new document section or anchor
  should be surfaced
- mark roadmap item `3.3.3` as done in [docs/roadmap.md](../roadmap.md) only
  after the implementation, tests, and docs are complete

Document any intentionally unsupported tracing features explicitly so the
boundary of `3.3.3` is clear.

## Stage 8: Run the repository gates and capture evidence

Run the full repository validation suite using `tee` and `set -o pipefail`,
respecting the repo note that `make typecheck` and `make test` must not run in
parallel because both rebuild `.venv`:

```bash
set -o pipefail && make fmt | tee /tmp/make-fmt-3-3-3.log
set -o pipefail && make check-fmt | tee /tmp/make-check-fmt-3-3-3.log
set -o pipefail && make lint | tee /tmp/make-lint-3-3-3.log
set -o pipefail && make typecheck | tee /tmp/make-typecheck-3-3-3.log
set -o pipefail && make test | tee /tmp/make-test-3-3-3.log
set -o pipefail && make markdownlint | tee /tmp/make-markdownlint-3-3-3.log
set -o pipefail && make nixie | tee /tmp/make-nixie-3-3-3.log
```

Expected outcome:

- all commands exit successfully
- the new Rust unit tests, Python behavioural tests, and snapshots are part of
  `make test`
- no doc-formatting drift remains after `make fmt`

If any gate fails, fix the root cause and rerun the failing gate plus any later
gates affected by the fix.

## Outcomes & Retrospective

This section remains intentionally incomplete until implementation finishes.
When the work is done, replace this placeholder with:

- the final public API shape
- the chosen feature-gating strategy
- what span data was ultimately supported
- the exact tests added
- the documentation files updated
- any lessons learned about Rust/Python integration through `tracing`
