# Implement roadmap item 3.4.2: improve structured logging in macros

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

## Purpose / Big Picture

Roadmap item 3.4.2 requires better structured logging ergonomics so context is
easy to propagate without repetitive boilerplate. The current
`femtolog_*!(...)` macros capture source location and format strings, but they
do not provide ergonomic structured field capture or context propagation.

After this work:

1. Rust callers can emit structured fields directly from logging macros and
   merge a caller-provided context map in one call.
2. Context propagation for macro-based logging is simplified through a scoped
   context API so call sites do not have to manually repeat shared fields.
3. Python behavioural coverage proves the structured metadata is preserved
   through the Rust-backed runtime and remains stable via snapshots.
4. The design and roadmap documents reflect the implemented contract.

Observable success: structured fields and propagated context appear in
`record.metadata.key_values` for accepted records, validation failures surface
as explicit Python/Rust errors, and `make fmt`, `make check-fmt`,
`make typecheck`, `make lint`, `make test`, `make markdownlint`, and
`make nixie` all pass.

## Constraints

- Preserve existing macro call forms:
  `femtolog_info!(logger, "msg")` and `femtolog_info!(logger, "{}", arg)`.
- Keep source-location capture behaviour unchanged (`file!`, `line!`,
  `module_path!`).
- Do not add new external dependencies.
- Keep compatibility with existing handler, formatter, and `log-compat` flows.
- Ensure no Python objects cross worker-thread boundaries; all structured
  fields must be Rust-owned before enqueueing.
- Use `rstest` for Rust unit coverage.
- Add Python behavioural tests via `pytest-bdd` and snapshots via `syrupy`.
- Update design docs when decisions are finalized.
- Mark roadmap item `3.4.2` done only after implementation and full validation.
- All files must remain under 400 lines; split modules when needed.

## Tolerances (Exception Triggers)

- Scope: if implementation needs more than 18 files changed (net), stop and
  escalate.
- API: if any existing public function/method signature must break
  compatibility, stop and escalate.
- Dependencies: if any new crate or Python package is required, stop and
  escalate.
- Test churn: if full-suite failures persist after 5 fix attempts, stop and
  escalate with failure summary.
- Ambiguity: if macro syntax alternatives are equally valid but materially
  different, pause and confirm before merging.

## Risks

- Risk: macro pattern expansion can become ambiguous with mixed
  format-args/structured-field syntax. Severity: medium Likelihood: medium
  Mitigation: add explicit macro arms and compile-fail/positive unit tests for
  each accepted and rejected form.

- Risk: context propagation may leak data across threads/tasks if storage is
  not scoped correctly. Severity: high Likelihood: low Mitigation: use scoped
  guard semantics plus thread/task isolation tests.

- Risk: structured field limits and key conflict rules may diverge from ADR 003
  enrichment constraints. Severity: medium Likelihood: medium Mitigation: reuse
  one shared validation path and document one canonical contract in design docs.

- Risk: Python snapshot tests can become flaky due to path/line variability.
  Severity: low Likelihood: medium Mitigation: normalize unstable fields before
  syrupy comparisons.

- **Technical Debt**: The `merge_context_values` and `active_context`
  implementations in `log_context.rs` lack a fast-path optimization for
  empty-context hot paths. Currently, every merge allocates and validates even
  when the context stack is empty. A future optimization should short-circuit
  allocation and validation when no scoped context exists. This is tracked as a
  known performance improvement for future work (referenced in roadmap item
  3.4.2 note and design §6.2, §8.3).

## Progress

- [x] (2026-03-04T00:00Z) Gather roadmap/design/testing context and draft this
      ExecPlan.
- [x] (2026-03-23T00:00Z) Finalize macro syntax and context propagation API
      surface.
- [x] (2026-03-23T00:00Z) Implement shared structured-field validation and
      metadata merge helper.
- [x] (2026-03-23T00:00Z) Implement macro updates and scoped context support.
- [x] (2026-03-23T00:00Z) Add Rust `rstest` unit tests for happy, unhappy, and
      edge paths.
- [x] (2026-03-23T00:00Z) Add Python `pytest-bdd` + `syrupy` tests for runtime
      behaviour.
- [x] (2026-03-23T00:00Z) Update design docs and mark roadmap item 3.4.2 done.
- [ ] Run full quality gates and capture evidence in this plan.

## Surprises & Discoveries

- Observation: existing macros already route through `log_with_metadata`, which
  gives a clean insertion point for structured key-value merging without
  reworking handler dispatch. Evidence: `rust_extension/src/logging_macros.rs`
  and `rust_extension/src/logger/mod.rs`. Impact: implementation can remain
  focused on metadata construction and validation, not dispatch internals.

- Observation: current Python BDD coverage for logging macros verifies formatted
  output only, not structured `metadata.key_values`. Evidence:
  `tests/features/logging_macros.feature` and
  `tests/steps/test_logging_macros_steps.py`. Impact: new behavioural scenarios
  must exercise structured record payloads via a handler that inspects
  `handle_record`.

## Decision Log

- Decision: scope this item to structured macro fields and scoped context
  propagation, not async handler runtime changes. Rationale: roadmap item 3.4.2
  and design §6.2 focus on macro ergonomics; design §8.3 is used for
  context-propagation safety considerations, not for introducing a new async
  runtime in this milestone. Date/Author: 2026-03-04 / Codex planning pass.

- Decision: validate structured field constraints through one shared helper used
  by both macro and Python-facing paths. Rationale: avoids drift between Rust
  and Python behaviour and keeps contract aligned with ADR 003-style enrichment
  rules. Date/Author: 2026-03-04 / Codex planning pass.

- Decision: include Python behavioural tests even though macros are Rust-facing.
  Rationale: this project is a Python library implemented in Rust; user-visible
  guarantees must be proven from Python test entrypoints. Date/Author:
  2026-03-04 / Codex planning pass.

## Outcomes & Retrospective

Pending implementation. This section will be completed after all milestones and
quality gates pass.

## Context and Orientation

`femtologging` is a Python logging library backed by a Rust extension crate.
The logging path creates a `FemtoLogRecord` on the producer side, attaches
`RecordMetadata`, and asynchronously dispatches to handlers.

Relevant files and roles:

- `rust_extension/src/logging_macros.rs`:
  Rust `femtolog_debug!`, `femtolog_info!`, `femtolog_warn!`, `femtolog_error!`
  routed through `__femtolog_at_level!` and split helper macro arms.
- `rust_extension/src/log_record.rs`:
  `RecordMetadata` with `key_values: BTreeMap<String, String>`.
- `rust_extension/src/logger/mod.rs`:
  `FemtoLogger::log_with_metadata` and dispatch/filter flow.
- `rust_extension/src/convenience_functions.rs`:
  Python module-level logging entrypoints and Python log-context bindings used
  for context propagation tests.
- `tests/features/logging_macros.feature` and
  `tests/steps/test_logging_macros_steps.py`: Existing BDD harness to extend.
- `docs/rust-multithreaded-logging-framework-for-python-design.md`:
  Design source for macro ergonomics (§6.2) and async/context direction (§8.3).
- `docs/configuration-design.md`:
  Existing enrichment constraints and compatibility contracts.
- `docs/roadmap.md`:
  checklist source where 3.4.2 must be marked done at completion.

## Plan of Work

### Stage A: Specify syntax and contracts (no production edits yet)

Define and lock the accepted macro call forms for structured fields and context
propagation. The minimum accepted syntax in this stage:

1. Existing forms remain valid.
2. Structured fields arm: `femtolog_info!(logger, "msg"; key = value, ...)`.
3. Combined format + structured fields arm:
   `femtolog_info!(logger, "{}", arg; key = value, ...)`.
4. Context spread arm (or equivalent) to merge pre-collected context with
   explicit fields, where explicit fields override duplicate keys.

Also define field validation contract and limits (key/value types, key
conflicts, max counts/sizes) and map the same rules to Python-facing behaviour
used in BDD tests.

Go/no-go:

- Go if syntax is unambiguous and representable with `macro_rules!`.
- No-go if syntax requires procedural macros or dependency additions.

### Stage B: Implement structured-field and context plumbing

Add a small internal module that:

1. Converts structured values into owned strings for
   `RecordMetadata.key_values`.
2. Merges scoped context and explicit per-call fields deterministically.
3. Applies centralized validation and returns typed errors.

Update `logging_macros.rs` macro arms to build metadata via this helper instead
of open-coding metadata structs. Keep the existing source-location capture
logic intact.

Add scoped context propagation API for Rust callers (guarded, nest-safe,
thread-safe) and ensure macro invocations read the active scoped context on the
producer thread.

Go/no-go:

- Go if existing macro behaviour and level filtering remain unchanged for
  non-structured calls.
- No-go if context storage leaks across test threads/tasks.

### Stage C: Testing (Rust unit + Python behavioural + snapshots)

Rust unit tests (`rstest`) must cover:

1. Happy path: structured fields are attached to metadata and survive dispatch.
2. Happy path: scoped context is inherited by macro calls.
3. Edge path: explicit fields override propagated context keys.
4. Unhappy path: invalid keys/values and limit breaches return clear errors.
5. Isolation path: nested scopes and concurrent threads do not leak context.

Python behavioural tests (`pytest-bdd`) must cover:

1. Happy path: Python-observable logs include structured metadata values.
2. Happy path: propagated context reaches emitted records.
3. Unhappy path: invalid structured context is rejected predictably.
4. Edge path: override precedence and empty-context behaviour.

Snapshot coverage (`syrupy`) must assert normalized structured payload output
for stable, reviewable behaviour.

Go/no-go:

- Go if new tests fail before implementation and pass after implementation.
- No-go if snapshot stability depends on machine-specific paths or timings.

### Stage D: Documentation, roadmap, and full validation

Update design documentation with finalized syntax, constraints, and context
propagation behaviour:

1. `docs/rust-multithreaded-logging-framework-for-python-design.md`:
   update §6.2 and §8.3 text to match implemented behaviour.
2. `docs/configuration-design.md`:
   align enrichment/context constraints with the shared helper contract.
3. `docs/roadmap.md`:
   mark `3.4.2` as done once implementation and tests are complete.

Run all required quality gates and record outcomes.

## Concrete Steps

Working directory: `/home/user/project`

1. Establish baseline and focused test failures for red/green flow.

   ```bash
   set -o pipefail && cargo test --manifest-path rust_extension/Cargo.toml \
     --no-default-features logging_macros::tests | tee /tmp/3-4-2-rust-red.log
   ```

2. Implement helper + macro updates, then run focused Rust tests.

   ```bash
   set -o pipefail && cargo test --manifest-path rust_extension/Cargo.toml \
     --features python -- logging_macros::tests convenience_functions::tests \
     | tee /tmp/3-4-2-rust-focused.log
   ```

3. Add/extend BDD scenarios and execute focused Python behavioural tests.

   ```bash
   set -o pipefail && uv run pytest \
     tests/features/logging_macros.feature \
     tests/steps/test_logging_macros_steps.py -q \
     | tee /tmp/3-4-2-python-bdd.log
   ```

4. Run required full gates before completing the work.

   ```bash
   set -o pipefail && make fmt | tee /tmp/3-4-2-fmt.log
   set -o pipefail && make check-fmt | tee /tmp/3-4-2-check-fmt.log
   set -o pipefail && make typecheck | tee /tmp/3-4-2-typecheck.log
   set -o pipefail && make lint | tee /tmp/3-4-2-lint.log
   set -o pipefail && make test | tee /tmp/3-4-2-test.log
   set -o pipefail && make markdownlint | tee /tmp/3-4-2-markdownlint.log
   set -o pipefail && make nixie | tee /tmp/3-4-2-nixie.log
   ```

Expected success markers:

- `make fmt`: exits 0.
- `make check-fmt`: exits 0.
- `make typecheck`: exits 0.
- `make lint`: exits 0 with no clippy warnings.
- `make test`: exits 0 with Rust and Python suites passing.
- `make markdownlint`: exits 0.
- `make nixie`: exits 0.

## Validation and Acceptance

Feature acceptance criteria:

1. Structured macro fields are present in `record.metadata.key_values`.
2. Scoped context propagation attaches shared fields without per-call
   duplication.
3. Explicit per-call fields override propagated context on key collisions.
4. Invalid keys/values/limits are rejected with deterministic errors.
5. Python BDD and syrupy snapshots prove behaviour from Python entrypoints.
6. Design docs and roadmap are updated to reflect completed behaviour.

Quality criteria:

- Tests: relevant Rust unit tests and Python BDD/snapshot tests pass.
- Lint/typecheck/format: required make targets pass.
- Compatibility: existing macro invocation forms remain valid.

## Idempotence and Recovery

- All commands in this plan are re-runnable.
- If snapshot assertions fail only due to unstable fields, normalize fields in
  step definitions rather than relaxing assertions.
- If a stage fails, revert only that stage's edits and rerun focused tests
  before resuming.
- Keep `/tmp/3-4-2-*.log` files as validation evidence until the change is
  merged.

## Artifacts and Notes

Implementation should produce:

- Updated Rust macro and context modules.
- New/updated Rust `rstest` cases for structured fields and propagation.
- Updated Python BDD feature/step files and syrupy snapshots.
- Updated design docs and roadmap checklist.
- Gate logs under `/tmp/3-4-2-*.log`.

## Interfaces and Dependencies

Planned interfaces (names may be adjusted during implementation, but behaviour
must remain equivalent):

```rust
pub fn log_with_metadata(
    &self,
    level: FemtoLevel,
    message: &str,
    metadata: RecordMetadata,
) -> Option<String>;
```

```rust
// Macro call shapes to support:
femtolog_info!(logger, "message");
femtolog_info!(logger, "{}", arg);
femtolog_info!(logger, "message"; key = value, other = value2);
femtolog_info!(logger, "{}", arg; key = value);
```

```plaintext
Python behavioural path:
- pytest-bdd scenarios in tests/features/logging_macros.feature
- syrupy snapshots in tests/steps/__snapshots__/
```

No new dependencies are allowed; use existing stdlib/Rust/PyO3 facilities.

## Revision Note

Initial draft created for roadmap item 3.4.2 planning based on
`docs/roadmap.md`, `docs/configuration-design.md`, and design sections §6.2 and
§8.3. Remaining sections are ready to be updated during implementation.
