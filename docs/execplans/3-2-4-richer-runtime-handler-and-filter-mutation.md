# Deliver richer runtime handler and filter mutation workflows

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item 3.2.4 asks for runtime reconfiguration that goes beyond
`FemtoLogger.set_level()`. After this change, Rust and Python callers should be
able to mutate the active handler and filter attachments of the root logger or
named loggers without restarting the process and without forcing a full
bootstrap rebuild.

The intended user-visible outcome is a Python-first control plane for targeted
runtime changes. A caller should be able to add a new handler, replace an
existing filter set, remove a previously attached handler by configuration ID,
or clear all filters on a logger, then observe the new behaviour immediately in
subsequent log calls. A failed mutation must leave the previous runtime state
intact.

Observable success after implementation:

1. Start with a logger that writes only to stderr and uses a level-based
   filter.
2. Apply a runtime mutation that adds a file handler and swaps in a different
   filter.
3. Emit a record and observe that the new file receives output and the new
   filter behaviour applies immediately.
4. Apply an invalid mutation referencing a missing handler or filter ID and
   observe that the previous runtime configuration still works unchanged.

This plan deliberately stops short of Python standard-library callback filter
parity. That work belongs to roadmap item 3.2.5 and must not be pulled into
3.2.4.

## Context and orientation

The current implementation already has pieces of runtime reconfiguration, but
they are not yet shaped into an explicit mutation workflow.

`rust_extension/src/logger/mod.rs` already supports:

- `set_level()` via `AtomicU8`
- `set_propagate()` via `AtomicBool`
- imperative handler attachment methods (`add_handler`, `remove_handler`,
  `clear_handlers`)
- imperative Rust-side filter methods (`add_filter`, `remove_filter`,
  `clear_filters`)

`rust_extension/src/config/build.rs` already rebuilds handlers and filters and
re-applies logger configuration through `ConfigBuilder.build_and_init()`.
However, that path is bootstrap-oriented. It always interprets handler and
filter collections as full replacement lists because `LoggerConfigBuilder` in
`rust_extension/src/config/types.rs` stores plain `Vec<String>` collections
rather than a tri-state “unchanged / clear / mutate” representation.

The second gap is identity. `FemtoLogger` stores handlers and filters as bare
`Arc<dyn FemtoHandlerTrait>` and `Arc<dyn FemtoFilter>` values. Once attached,
the system no longer knows which configuration ID produced which runtime
object. That makes an explicit “remove handler `audit`” workflow impossible
without additional manager-side metadata.

The files most relevant to this work are:

- `rust_extension/src/config/build.rs`
- `rust_extension/src/config/mod.rs`
- `rust_extension/src/config/types.rs`
- `rust_extension/src/config/types/python_bindings.rs`
- `rust_extension/src/logger/mod.rs`
- `rust_extension/src/manager.rs`
- `rust_extension/src/filters/mod.rs`
- `femtologging/_femtologging_rs.pyi`
- `femtologging/config_protocol.py`
- `tests/test_filters.py`
- `tests/features/dynamic_level.feature`
- `tests/steps/test_dynamic_level_steps.py`

Two file-size constraints matter immediately:

- `rust_extension/src/config/types.rs` is already close to the 400-line limit.
- `rust_extension/src/logger/mod.rs` is already well beyond the preferred file
  size and must not absorb more substantial logic.

New runtime-mutation code should therefore be split into new modules instead of
being appended to those files.

## Constraints

- Keep the scope on runtime mutation of handler and filter attachments plus any
  supporting level/propagate updates needed for a coherent workflow.
- Do not implement Python stdlib callback filters, dictConfig `"()"`
  filter-factory support, or contextvar enrichment here. Those belong to
  roadmap item 3.2.5 and its sub-items.
- Do not introduce new external Rust or Python dependencies.
- Preserve existing `ConfigBuilder.build_and_init()` semantics for bootstrap
  configuration.
- Avoid storing Python objects or any GIL-bound state in worker threads. The
  feature must remain compatible with the guidance in
  `docs/multithreading-in-pyo3.md`.
- Public Python-facing APIs must receive matching stubs in
  `femtologging/_femtologging_rs.pyi`.
- New Rust tests must use `rstest` fixtures and parameterized cases where
  repetition would otherwise appear, following
  `docs/rust-testing-with-rstest-fixtures.md`.
- New public Rust APIs must receive Rustdoc examples or a justified doctest
  strategy following `docs/rust-doctest-dry-guide.md`.
- `make check-fmt`, `make typecheck`, `make lint`, `make test`,
  `make markdownlint`, and `make nixie` must all pass before the work is
  considered complete.
- Update both `docs/rust-multithreaded-logging-framework-for-python-design.md`
  and `docs/configuration-design.md` with the final design decisions, and mark
  roadmap item 3.2.4 as done only after implementation and validation finish.

## Tolerances (exception triggers)

- Scope: if the implementation needs changes in more than 20 files or roughly
  900 net lines of code and documentation, stop and escalate with a reduced
  slice.
- Interface: if preserving existing `ConfigBuilder` and `LoggerConfigBuilder`
  semantics becomes impossible, stop and escalate instead of repurposing them.
- Feature boundary: if the design starts requiring Python callback filters or
  formatter hot-swapping to make 3.2.4 useful, stop and escalate because that
  means the roadmap items are entangled incorrectly.
- Concurrency: if safe mutation requires changing handler worker-thread
  contracts in more than four handler modules, stop and escalate.
- Validation: if any required gate still fails after five focused fix attempts,
  stop and document the blocker.

## Risks

- Risk: the current system has no persistent mapping from configuration IDs to
  live handler or filter objects. Mitigation: add manager-owned runtime state
  that records active handler IDs, filter IDs, and the corresponding shared
  `Arc` registries without adding ID lookups to the hot path.

- Risk: partial runtime mutation could leave a logger half-updated if errors
  are detected after some swaps have already happened. Mitigation: build and
  validate all new handlers and filters first, compute the full post-mutation
  state in memory, then apply via snapshot-and-commit with rollback data for
  every touched logger.

- Risk: unchanged handlers such as file, rotating-file, socket, or HTTP
  handlers may be unnecessarily torn down and recreated, causing avoidable
  disruption. Mitigation: preserve live handler `Arc`s for unchanged IDs and
  rebuild only IDs explicitly added or replaced by the mutation request.

- Risk: lock ordering between the manager, PyO3 attachment, and logger-local
  `RwLock`s could introduce deadlocks. Mitigation: follow the current
  `build_and_init()` structure by building objects before entering
  `Python::attach`, keep GIL-held mutation windows short, and avoid holding the
  manager lock while performing any blocking handler shutdown work.

- Risk: Python behavioural tests can overmatch step text when new scenarios
  resemble existing ones. Mitigation: prefer anchored regex step definitions
  when step text overlaps, matching the existing project note about
  `pytest-bdd` parser overmatching.

## Proposed design

Introduce a dedicated runtime-mutation API instead of overloading
`LoggerConfigBuilder`.

The top-level API should be a new `RuntimeConfigBuilder` with Python and Rust
parity. It owns:

- optional builder definitions for newly introduced or replaced handlers
- optional builder definitions for newly introduced or replaced filters
- one mutation entry for the root logger
- zero or more mutation entries for named loggers

Each target logger should use a `LoggerMutationBuilder` rather than
`LoggerConfigBuilder`. This builder must encode explicit collection mutation
semantics for handlers and filters:

- unchanged
- replace with a specific list of IDs
- append specific IDs
- remove specific IDs
- clear the collection

The internal representation should be a dedicated enum such as
`CollectionMutation<T>`, not ad hoc combinations of booleans and vectors. This
solves the tri-state problem cleanly and keeps Python snapshot output stable
through `as_dict()`.

The manager should gain runtime-state tracking. At minimum it must retain:

- active shared handler objects by configuration ID
- active shared filter objects by configuration ID
- per-logger attachment metadata listing handler IDs and filter IDs currently
  attached through the structured configuration path

This manager-owned metadata is the source of truth for “remove handler
`audit`”, “append filter `warn_only`”, and “preserve existing handler `stderr`
without rebuilding it”. `FemtoLogger` itself should remain hot-path focused and
continue storing plain `Arc` collections for fast evaluation and dispatch.

The runtime mutation API should be transactional at the library level:

1. Validate the mutation request.
2. Build any new handler or filter IDs before touching live state.
3. Compute the full post-mutation attachment lists for every targeted logger.
4. Snapshot the current logger and manager runtime state.
5. Apply the new state.
6. If any step in the commit path fails, restore the snapshots and return the
   error.

Runtime mutation should remain scoped to Rust-backed builder-defined handlers
and filters. Ad hoc Python handler objects attached through
`FemtoLogger.add_handler()` remain supported as imperative operations but are
outside the structured reconfiguration control plane for this roadmap item.

## Plan of work

### Stage A: Add a dedicated runtime-mutation model

Create a new config submodule rather than expanding
`rust_extension/src/config/types.rs`. A good starting shape is:

- `rust_extension/src/config/runtime_mutation.rs`
- `rust_extension/src/config/runtime_mutation/python_bindings.rs`

Define:

- `RuntimeConfigBuilder`
- `LoggerMutationBuilder`
- `CollectionMutation<String>`
- any error types needed for invalid mutation combinations

Expose Python bindings with the same builder style already used elsewhere in
the project. The Python surface should be chainable and snapshot-friendly. Add
`as_dict()` support for the new builders so BDD and syrupy tests can snapshot
the mutation request shape without inspecting Rust internals.

Acceptance for this stage:

- the new builders compile
- they round-trip through Python bindings
- invalid combinations such as “append handlers then replace handlers in the
  same builder” are rejected deterministically

### Stage B: Teach the manager about live reconfiguration state

Extend `rust_extension/src/manager.rs` with a runtime-state structure that
tracks:

- shared handlers by ID
- shared filters by ID
- per-logger attachment metadata

Update the bootstrap path in `rust_extension/src/config/build.rs` so
`ConfigBuilder.build_and_init()` seeds and refreshes this runtime-state data
whenever structured configuration is applied. Also update
`disable_existing_loggers()` and `reset_manager()` so the new metadata cannot
go stale across tests or rebuilds.

Acceptance for this stage:

- initial configuration and reconfiguration tests still pass
- manager state reflects the bootstrap configuration accurately
- unchanged handlers and filters can be preserved by ID without rebuilding

### Stage C: Implement transactional runtime apply

Add an `apply()` method on `RuntimeConfigBuilder`. Keep the structure parallel
to `ConfigBuilder.build_and_init()`:

1. Build any newly declared handlers and filters outside `Python::attach`.
2. Enter `Python::attach` only for logger lookup and state commit.
3. Resolve every target logger through `manager::get_logger`.
4. Compute the final handler/filter ID lists by applying
   `CollectionMutation<String>` against manager-owned runtime metadata.
5. Swap logger attachments using dedicated helper methods extracted out of
   `rust_extension/src/logger/mod.rs` into a new module so the existing file
   does not grow further.
6. Commit updated manager metadata only after logger state has been applied
   successfully.

The logger-side helpers should support efficient whole-collection replacement,
not repeated clear/add loops. Add replacement helpers in a new logger submodule
such as `rust_extension/src/logger/runtime_mutation.rs` and keep
`FemtoLogger`’s hot-path behaviour unchanged.

Acceptance for this stage:

- append, replace, remove, and clear workflows work for handlers and filters
- unchanged IDs preserve their live objects
- invalid IDs fail before any logger is mutated
- rollback leaves the previous runtime state intact on failure

### Stage D: Expose the Python-first control plane

Wire the new builders into the Python module and typing surface:

- register the new classes in `rust_extension/src/python_module.rs`
- re-export them in `rust_extension/src/lib.rs` if the Rust API should be
  public
- update `femtologging/_femtologging_rs.pyi`
- update `femtologging/config_protocol.py` if the protocol needs to model the
  new builder type
- update `femtologging/__init__.py` if the new classes are meant to be
  top-level imports

The Python naming should remain explicit. Prefer `RuntimeConfigBuilder` and
`LoggerMutationBuilder` over shorter names that blur bootstrap configuration
with runtime mutation.

Acceptance for this stage:

- Python users can build and apply a runtime mutation without dropping into
  Rust-only APIs
- type checking knows about the new builders and their methods
- public docs and docstrings use Python-first examples

### Stage E: Validate with Rust, Python, BDD, and snapshots

Rust unit tests should live in new focused test modules instead of further
growing the existing large files. Use `rstest` fixtures and parameterized cases.

Add Rust coverage for:

- append/replace/remove/clear for handlers
- append/replace/remove/clear for filters
- preserving unchanged shared handler `Arc`s across runtime mutations
- failure on unknown IDs with previous runtime state preserved
- root logger mutations and named logger mutations
- interaction with `set_level()` and `set_propagate()` when those fields are
  included in a runtime mutation

Add Python unit tests for direct API usage, likely in a new file such as
`tests/test_runtime_reconfiguration.py`.

Add a BDD feature such as `tests/features/runtime_reconfiguration.feature` with
scenarios for:

- appending a handler at runtime
- replacing filters at runtime
- clearing filters to re-enable emissions
- failed mutation preserving the prior state
- root logger mutation

Add syrupy snapshots for:

- the runtime-mutation builder `as_dict()` output
- a normalized post-apply state object captured in the BDD steps

Keep unhappy-path scenarios explicit. This feature is not complete unless
failure paths are exercised in both Rust and Python.

### Stage F: Document the final design and close the roadmap item

Update `docs/rust-multithreaded-logging-framework-for-python-design.md` section
7.2 to record the accepted runtime mutation strategy:

- dedicated runtime-mutation builders
- manager-owned live attachment metadata
- transactional apply semantics
- scope boundary excluding stdlib callback filters and formatter hot-swapping

Update `docs/configuration-design.md` section 3 with the concrete API and the
collection mutation semantics.

After the implementation, tests, and docs are complete, mark roadmap item 3.2.4
as done in `docs/roadmap.md`.

## Validation checklist

Run these commands exactly, keeping full logs for later inspection:

```plaintext
set -o pipefail && make fmt | tee /tmp/3-2-4-fmt.log
set -o pipefail && make check-fmt | tee /tmp/3-2-4-check-fmt.log
set -o pipefail && make typecheck | tee /tmp/3-2-4-typecheck.log
set -o pipefail && make lint | tee /tmp/3-2-4-lint.log
set -o pipefail && make test | tee /tmp/3-2-4-test.log
set -o pipefail && make markdownlint | tee /tmp/3-2-4-markdownlint.log
set -o pipefail && make nixie | tee /tmp/3-2-4-nixie.log
```

Expected outcome:

- all commands exit successfully
- Rust tests show new `rstest`-based runtime reconfiguration coverage
- Python tests include new `pytest-bdd` scenarios and syrupy snapshots
- no existing dynamic-level or filter tests regress

## Progress

- [x] 2026-03-11: Wrote the initial ExecPlan draft after reviewing the roadmap,
  design docs, runtime reconfiguration code, and existing tests.
- [ ] Confirm the final public API names before implementation starts.
- [ ] Add runtime-mutation builder types and Python bindings.
- [ ] Extend manager runtime state and bootstrap seeding.
- [ ] Implement transactional runtime apply and rollback.
- [ ] Add Rust `rstest` coverage.
- [ ] Add Python unit, `pytest-bdd`, and syrupy coverage.
- [ ] Update design docs and mark roadmap item 3.2.4 done.
- [ ] Run all quality gates and keep the logs.

## Surprises & Discoveries

- The current codebase already supports a limited form of handler and filter
  replacement through repeated `ConfigBuilder.build_and_init()` calls, but that
  path is implicit and bootstrap-shaped rather than an explicit runtime
  mutation workflow.
- `LoggerConfigBuilder` cannot represent “leave handlers unchanged” because its
  handler and filter collections are stored as plain vectors.
- The manager currently tracks only logger objects. It does not remember which
  configuration IDs produced the attached handlers or filters, so an ID-based
  runtime mutation API needs new manager-side state.
- `rust_extension/src/config/types.rs` is near the 400-line project limit and
  `rust_extension/src/logger/mod.rs` is already far beyond it, so this feature
  must be split into new modules from the start.

## Decision Log

- Decision: use a dedicated `RuntimeConfigBuilder` and
  `LoggerMutationBuilder` instead of extending `LoggerConfigBuilder`.
  Rationale: the existing bootstrap builders cannot cleanly distinguish
  unchanged state from explicit clearing, and preserving their semantics avoids
  subtle regressions in `build_and_init()`. Date/Author: 2026-03-11 / Codex.

- Decision: keep live handler/filter identity in manager-owned runtime state
  rather than on `FemtoLogger`. Rationale: the hot path should continue using
  plain `Arc` collections for fast dispatch and filter evaluation, while the
  control plane needs ID-aware metadata for append/remove workflows.
  Date/Author: 2026-03-11 / Codex.

- Decision: treat runtime mutation as a transactional operation with
  build-first, swap-second semantics. Rationale: runtime reconfiguration is
  only useful if failure does not leave the logger graph in a partially updated
  state. Date/Author: 2026-03-11 / Codex.

- Decision: keep formatter hot-swapping out of scope for 3.2.4.
  Rationale: the roadmap item explicitly calls out handler and filter mutation,
  and widening the scope would conflate this work with the more general future
  reconfiguration ideas in design section 7.2. Date/Author: 2026-03-11 / Codex.

## Outcomes & Retrospective

This document is still in draft status. No code has been implemented yet, no
roadmap entry has been marked done, and no design document text has been
updated beyond this plan. The intended outcome of execution is a Python-first,
transactional runtime mutation API for handler and filter attachments that is
fully covered by Rust unit tests, Python behavioural tests, and snapshots.
