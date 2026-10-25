# Prune unreferenced runtime handler and filter definitions after mutation

This Execution Plan (ExecPlan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IN PROGRESS

## Purpose / big picture

`RuntimeConfigBuilder.apply()` currently constructs every handler and filter
definition it is given, merges the constructed objects into the previous global
runtime registries, and publishes the merged maps through
`manager::replace_runtime_state()`. Nothing checks whether the resulting logger
attachment graph can reach those definitions. A mutation that supplies a fresh,
never-attached file handler identifier therefore succeeds and keeps the
handler, its worker thread, queue, and open file descriptor alive for the
lifetime of the manager.

After this change, reachability from the committed logger attachment graph
becomes the invariant for both registries. The graph includes filters named by
attached handlers as well as filters attached directly to loggers. A supplied
definition that no logger in the resulting state can reach is rejected with
`InvalidMutation` *before* any object is constructed, so no file is created, no
socket is opened, and no worker thread is spawned. Definitions that become
unreachable because a mutation removed, replaced, or cleared their last
reference are dropped from the published registries, and their objects are
released once in-flight records have drained and their remaining references
end. Bootstrap configuration (`ConfigBuilder.build_and_init()`) applies the
same rule to what it publishes, while continuing to build and validate every
definition it is given.

A user can observe the change without reading any Rust:

1. Apply a mutation that only declares an unused file handler. `apply()` raises
   `ValueError` naming the identifier, and the target file is never created.
2. Repeat that 32 times with fresh identifiers. The runtime registry length
   stays constant and no files appear in the temporary directory.
3. Attach a handler, then remove its last reference. The file handler's worker
   thread is joined and the object is released once the logger's queue is
   drained.

## Context and orientation

The relevant code lives under `rust_extension/src/config/`.

`RuntimeConfigBuilder` (in `rust_extension/src/config/runtime_mutation.rs`)
holds maps of handler and filter *definitions* keyed by identifier, plus one
`LoggerMutationBuilder` for the root logger and zero or more for named loggers.
`RuntimeConfigBuilder::apply()` validates the request, takes the configuration
transaction lock, snapshots manager state, and plans the new logger
attachments. It rejects unknown or unreachable definitions before building
objects, then assembles and applies the pruned commit.

`rust_extension/src/config/runtime_mutation/apply.rs` owns orchestration and
pre-build validation. `commit.rs` plans logger attachment IDs with `KnownIds`,
constructs definitions only after reachability succeeds, then merges and prunes
the registries before applying the commit. Its handler-filter ID map carries
the additional graph edge from an attached handler to its filters.
`apply_commit()` updates impacted live loggers and publishes the pruned
registries through `manager::replace_runtime_state()`.

`rust_extension/src/config/runtime_mutation/validation.rs` holds the shared
validation helpers: `collection_conflict()`, `resolve_attachment_ids()`, and
`validate_remove_ids()`.

`rust_extension/src/config/build.rs` implements the bootstrap path,
`ConfigBuilder.build_and_init()`. It builds every declared handler and filter,
prepares a plan per logger, stages and commits loggers, applies the plans, and
publishes only reachable handlers, their filter-ID mappings, and the filters
reachable through loggers or attached handlers.

`rust_extension/src/manager.rs` owns the global manager. It stores a
`LoggerAttachmentState` (handler identifiers and direct filter identifiers
recorded for one logger), a `RuntimeStateSnapshot` (both registries, each
handler's filter identifiers, and the per-logger states), and the
`snapshot_runtime_state()` / `replace_runtime_state()` functions used by both
publication paths. The manager is guarded by a `parking_lot::RwLock` and the
file states that access must happen while the Python Global Interpreter Lock
(GIL) is held.

Concrete handlers acquire resources when they are constructed.
`FemtoFileHandler` opens or creates its file and spawns a worker thread;
`FemtoSocketHandler` and `FemtoHTTPHandler` spawn workers and own queues. All
three free those resources in `Drop`, which calls a `close()` that joins the
worker with a bounded timeout.

Two terms are used throughout this plan. A *definition* is a builder supplied
to a configuration API under an identifier. A *published registry entry* is the
live object recorded under that identifier in the manager's runtime state after
a successful commit. *Reachable* means the committed logger attachment metadata
reaches the definition directly or, for a filter, through an attached handler's
configured filter IDs.

## Constraints

- `manager.rs` must remain functionally unchanged. No new public registry
  inspection API, and no change to `replace_runtime_state()` or
  `snapshot_runtime_state()` semantics, are in scope.
- Existing error semantics must be preserved. `ConfigError::UnknownIds` keeps
  its precedence over the new reachability rejection, and keeps the order of
  the identifiers it reports.
- Runtime mutation stays sequential. No rollback machinery, no unregister
  operation, and no dormant-definition support are added.
- The hot path stays untouched. Neither `FemtoLogger`'s dispatch path nor the
  handler implementations change.
- No new external Rust or Python dependency.
- No file may exceed 400 lines.
- Comments and documentation use en-GB-oxendict spelling.
- Public Python-facing APIs keep matching stubs in
  `femtologging/_femtologging_rs.pyi`.
- Every Rust module begins with a `//!` comment; new public items carry Rustdoc.
- The manager lock must not be held across handler or filter construction.
- Existing tests must keep passing. In particular the bootstrap path must keep
  building and validating *all* declared definitions, and must keep configuring
  only the loggers named in the configuration.

## Tolerances (exception triggers)

- Scope: if the change needs more than 14 files or roughly 900 net lines, stop
  and escalate with a reduced slice.
- Interface: if `ConfigBuilder`, `LoggerConfigBuilder`, `RuntimeConfigBuilder`,
  or `LoggerMutationBuilder` needs a changed or removed method signature, stop
  and escalate.
- Semantics: if preserving `UnknownIds` precedence over the new rejection turns
  out to be impossible without an extra public error variant, stop and escalate.
- Dependencies: if a new external dependency is required, stop and escalate.
- Iterations: if a required gate still fails after five focused fix attempts,
  stop and document the blocker.
- Verification: if the `Weak`-based lifetime tests cannot be made deterministic
  within the existing bounded close timeouts, stop and escalate rather than
  weakening them into tautologies.

## Risks

- Risk: the existing Python suite may rely on declaring a handler or filter that
  no logger attaches, which the new rejection would turn into a failure.
  Severity: medium. Likelihood: medium. Mitigation: survey `tests/` before
  implementing (prerequisite P1 below) and record every affected call site in
  the Decision Log.

- Risk: the repository layering may not permit `config::build` and the runtime
  mutation module to share one helper module. Severity: low. Likelihood: low.
  Mitigation: verify with a compile of the module (prerequisite P2 below)
  before restructuring anything.

- Risk: a lifetime assertion about a replaced handler could be satisfied for the
  wrong reason - for example, the handler is released because a `Drop` ran
  elsewhere, or the assertion passes because the temporary directory outlives
  the worker. Severity: high. Likelihood: medium. Mitigation: use
  `Arc::downgrade()` on the published registry entry itself and assert both
  directions - the object is still alive while an in-flight record holds it,
  and gone once the record has drained. Assert registry length separately so
  the two properties cannot mask each other.

- Risk: the reachability rule could drop a handler that a live logger still
  needs, because a logger's attachments were made imperatively. Severity:
  medium. Likelihood: low. Mitigation: reachability is computed from the
  *whole* committed attachment map, not just from the loggers named in the
  mutation, so every logger with recorded metadata keeps its entries. Document
  that imperative `add_handler()` attachments are outside the structured
  control plane and hold their own references.

- Risk: pruning in the bootstrap path could change the behaviour of
  `disable_existing_loggers()`. Severity: medium. Likelihood: low. Mitigation:
  pruning only removes registry entries, and only after the logger plans have
  been applied and the per-logger attachment map has been replaced. It cannot
  affect which loggers are configured or which live attachments they hold.

## Progress

- [x] (2026-10-16) Read the runtime mutation, commit, validation, bootstrap,
  manager, logger, and handler modules; recorded acceptance criteria and the
  open questions the plan must resolve.
- [x] (2026-10-16) Renamed the branch to
  `issue-434-prune-unreferenced-runtime-handler-and-filter-definitions-after-mutation`
  and pushed it as the tracking branch, and retitled the session.
- [x] (2026-10-16) Prerequisite P1: surveyed Python and Rust tests for
  definitions that no logger attaches. Blast radius is small and bounded; see
  `Surprises & Discoveries`. No test asserts on registry size or key sets.
- [x] (2026-10-16) Prerequisite P2: confirmed `config::reachability` is
  reachable from both `config::build` and `config::runtime_mutation`; all three
  are `#[cfg(feature = "python")]` and `lib.rs` lists `pub mod config` first.
- [x] (2026-10-16) Prerequisite P3: recorded the pre-change behaviour of
  `apply()` from the existing code: it built every supplied definition before
  any reachability notion existed, so the unused-definition case could not fail
  for a structural reason. The regression tests written in Stage D are the red
  evidence: each asserts the rejection (or the prune) that the pre-change code
  did not produce.
- [x] (2026-10-16) EP-M1: added `config/reachability.rs`, registered it in
  `config/mod.rs`, reordered `apply()` to validate, snapshot, plan, reject
  unreachable definitions, and only then build; split `prepare_commit()` into
  `plan_mutation()` plus `assemble_commit()`; added `KnownIds` to
  `validation.rs`. `runtime_mutation.rs` and `runtime_mutation_tests.rs` both
  exceeded the 400-line project limit and were split by extracting
  `runtime_mutation/logger_mutation.rs` and `runtime_pruning_tests.rs`.
- [x] (2026-10-16) EP-M2: `build_and_init()` now prunes both registries
  against the applied logger plans immediately before
  `replace_runtime_state()`. Every declared definition is still built and
  validated first, so bootstrap keeps rejecting declaration errors for
  unattached definitions.
- [x] (2026-10-16) EP-M3: added `reachability_tests.rs` (9 unit tests),
  `runtime_pruning_tests.rs` (9 identifier-level regression tests),
  `runtime_lifecycle_tests.rs` (5 lifetime and churn tests, including the gated
  in-flight record), shared scaffolding in `test_utils::prune_test_support`,
  and 5 Python tests in `tests/test_runtime_reconfiguration.py`.
- [x] (2026-10-16) EP-M4: documented the lifecycle rule in
  `docs/configuration-design.md` section 3.2, cross-referenced it from the
  3.2.4 ExecPlan, and updated the `apply()` Rustdoc and the Python type stub.
- [x] (2026-10-16) Added the BDD scenario
  `A mutation declaring an unattached handler is rejected` to
  `tests/features/runtime_reconfiguration.feature` with its two steps, so the
  `ValueError` path is exercised through the same suite that already covers the
  `KeyError` path.
- [x] (2026-10-16) Gate `make check-fmt`, `make markdownlint`, and
  `make nixie` pass on the current tree. `make lint` passed every Python stage
  and five of six Rust clippy lanes, then stopped on the machine-wide
  `build-limits` supervisor outage described in `Surprises & Discoveries`.
- [x] (2026-10-16) `make lint` fully green except for the Whitaker Dylint step,
  which reported one finding in the new code:
  `Module prune_test_support must start with an inner doc comment.` Evidence:
  the `-11` log, which passed all seven `lint-env-policy` lanes, all five
  `-D warnings` `lint-rust-clippy` lanes, and then failed `lint-rust` at the
  final Dylint command with `LINT_EXIT=2` and a single
  `--> src/config/test_utils.rs:21:5` pointer. Clippy is therefore **verified
  clean**, not merely unrefuted: the strict lanes compile with `-D warnings`,
  unlike the relaxed policy lanes.
- [x] (2026-10-16) Fixed the Dylint finding by moving that module's doc comment
  inside the `mod` body as a `//!` block, which is what the repository rule
  ("Every module **must** begin with a module level (`//!`) comment") requires.
  The outer `///` form does not satisfy it, because the rule inspects the first
  item *inside* the module.
- [x] (2026-10-23) Audited the resumed worktree and found the scope tolerance
  exceeded: there are 14 modified tracked paths and 6 untracked paths,
  including this plan. Excluding the plan, the implementation spans 19 paths
  and about 1,435 net added lines. Paused at the tolerance as required.
- [x] (2026-10-23) The user explicitly authorized an exception to the scope
  tolerance and directed continuation. Continue the full acceptance criteria,
  including HTTP churn, by adding `HTTPHandlerBuilder` to the existing config
  builder union and Python extraction without changing method signatures.
- [x] (2026-10-23) First full gate pass: format, Markdown lint, and Mermaid
  checks passed. Python lint stages passed, while the Rust lint build failed
  with missing vendored OpenSSL/Cargo output paths. Type checking found the
  missing `HandlerBuilder::Http` arm in `as_pydict()`; added it, then the test
  build failed on the same compile error before the fix. Rerun every gate on
  the corrected candidate before review.
- [x] (2026-10-24) The fresh full gate run passed `make check-fmt`, `make lint`,
  `make typecheck`, `make test`, `make markdownlint`, and `make nixie`. The
  test run passed 744 Python tests, skipped 5, and passed all 66 snapshots; all
  Rust feature matrices, trybuild suites, integration tests, and doctests
  passed.
- [x] (2026-10-24) The BDD scenario
      `A mutation declaring an unattached handler is rejected` ran successfully
      as part of the full test gate.
- [x] (2026-10-25) Addressed CodeRabbit's review findings: use
  `with_endpoint` in the Python test, structure the type-stub errors under a
  NumPy-style `Raises` section, fix two broken prose lines, parameterize
  resource-handler churn tests, and share stream-target selection in the BDD
  steps while preserving case-insensitive matching and the unsupported-target
  error.
- [x] (2026-10-25) Final validation passed `make check-fmt`, `make lint`,
  `make typecheck`, `make test`, `make markdownlint`, and `make nixie`. Pytest
  passed 744 tests, skipped 5, and passed all 66 snapshots; all Rust feature,
  trybuild, integration, and doctest suites passed.
- [x] (2026-10-25) The final CodeRabbit review covered all 24 changed paths
  and reported no findings. After its initial rate limit, the single retry ran
  after the requested 61-minute `vsleep` cooldown.
- [x] (2026-10-25) Rebased the single issue commit from
  `abc25fbdca91ba86c0e0ec37812f912cf722099c` (exclusive base
  `27ea9feffb1452f626531f75024f2e20e7484b0c`) onto
  `d234990f7f4c3a7c000215cf9b90ad87d5d43c25`. Preserved the target's extracted
  runtime modules, formatter/HTTP builders, and handler-filter ID metadata.
  Extended reachability through attached handlers and added a regression that
  prunes a filter after its last handler detaches. The rebased tree still needs
  its full gates and CodeRabbit review.
- [x] (2026-10-25) Split the added Python reachability and lifecycle tests into
  `tests/test_runtime_definition_lifecycle.py` after Pylint found the expanded
  reconfiguration test module exceeded its 400-line limit. The original module
  is now 358 lines; full gates are being rerun on the updated candidate.
- [x] (2026-10-25) Fixed the other findings from post-rebase gate attempts:
  moved annotation-only imports under `TYPE_CHECKING`, documented the private
  Rust mutation fields and helpers, and reused the shared
  `configured_core_logger` fixture instead of redefining it locally. The gated
  worker test checks the result from `add_handler`, and its test-support enum
  and fields are documented.
- [x] (2026-10-25) All six deterministic gates passed on tree
  `31bae531e52041cecf723b74d4b343b833f611e5`: check-fmt, lint, typecheck, test,
  markdownlint, and nixie. Pytest reported 788 passed, 5 skipped, and all 66
  snapshots passed. Its single `PytestAssertRewriteWarning` also appears in
  earlier logs and points to the unchanged logging-context plugin registration.
- [x] (2026-10-25) Resolved the local/remote divergence. Local `53d88b4` and
  remote `abc25fb` are not ancestors of one another despite an identical
  subject line. The remote head is the **pre-rebase** commit, preserved locally
  as `refs/backup/issue-434-rebase/old-head-abc25fb`; its parent is the old base
  `27ea9fe`, not current main `d234990`. Local `53d88b4` is the rebased head.
  Nothing was lost in the rebase: the four paths `abc25fb` touched beyond the
  issue's own file set (`config/py.rs`, `config/types.rs`,
  `config/types/python_bindings.rs`,
  `tests/test_env_access_policy_contract.py`) are all upstream work that main
  already carries — main's `config/py.rs` already imports `HTTPHandlerBuilder`
  three times, and the `python_bindings.rs` content is byte-identical between
  `abc25fb` and main. The rebased file lengths match this worktree exactly
  (`reachability.rs` 200, `reachability_tests.rs` 190,
  `runtime_pruning_tests.rs` 300), confirming the working tree is the rebased
  tree. Remote head must therefore be force-pushed with the rebased commit; the
  old head is recoverable from the backup ref.
- [x] (2026-10-25) Confirmed that tree `31bae53`, recorded above as passing all
  six gates, is HEAD's tree with only a 6-line progress note added to this
  plan. The confirming gate run is therefore a re-verification of an
  already-green tree, not a first-pass attempt.
- [x] (2026-10-25) CodeRabbit reviewed 21 files at local HEAD
  `c0dd2f712d34c2dec7a699b9f8dcc8f7547a0510` and found one trivial duplicate
  pytest-fixture issue. Moved `reset_logger_state` and `core_config` to
  `tests/conftest.py`; module-level `pytestmark` keeps reset isolation scoped
  to the two owning test modules. `make fmt` and all six deterministic gates
  then passed on the three-file repair: 788 pytest tests passed, 5 were
  skipped, all 66 snapshots passed, and every Rust feature lane passed. The
  only warning was the previously observed upstream
  `PytestAssertRewriteWarning` in `tests.steps.logging_context_steps`.
- [ ] Draft pull request to be opened.

## Surprises & Discoveries

- Observation: `RuntimeConfigBuilder::new().apply()` is currently a successful
  no-op that republishes the merged registries unchanged. Because reachability
  is computed from the whole attachment map rather than from the mutated
  loggers alone, it stays a no-op after pruning rather than emptying both
  registries. Evidence: `collect_impacted()` and `logger_states.clone()` in
  `rust_extension/src/config/runtime_mutation.rs`. Impact: the empty-mutation
  regression test asserts *unchanged* registries, which is a stronger and more
  meaningful assertion than "does not fail".

- Observation: bootstrap publication currently publishes every declared
  definition, even one no logger references. Impact: without the bootstrap half
  of the rule, the `RuntimeConfigBuilder` path could inherit an unreachable
  entry and re-prune it on the next mutation, but a bootstrap-only deployment
  would still retain it indefinitely.

- Observation (P1, 2026-10-16): no test in the repository asserts on the *size*
  or key set of either registry. `handler_registry` and `filter_registry` are
  `pub(crate)` and the only test that reads a snapshot
  (`config_tests.rs:282-291`) inspects `logger_states`, not the registries.
  `_runtime_attachment_state_for_test` returns per-logger identifier lists, and
  `handler_ptrs_for_test` reads the live logger's handler vector; pruning
  changes neither. Evidence: exhaustive grep over `tests/` and
  `rust_extension/src/**`. Impact: the bootstrap prune cannot break an existing
  assertion by shrinking a registry.

- Observation (P1, 2026-10-16): the same survey found several existing call
  sites that declare a handler or filter nothing attaches, all of which go
  through `build_and_init()` and therefore keep succeeding under
  build-all/publish-reachable. They are
  `tests/steps/test_dynamic_level_steps.py:52`,
  `tests/steps/test_propagate_steps.py:181`,
  `tests/test_config_transaction_property.py:69-77` (Hypothesis can generate an
  empty attachment set), and several `dictConfig` cases that raise before
  publication for unrelated reasons. Impact: bootstrap must not reject
  unreachability, which is what the plan already specifies. No Python test
  reaches `apply()` with an unreferenced definition.

- Observation (P1, 2026-10-16): `builder_with_root` in
  `rust_extension/src/config/config_tests.rs:15-19` declares handler `"h"` with
  a root logger that attaches nothing. It is used, directly or through
  `base_logger_builder`, by ten tests; in each case where the builder is
  actually built, some logger attaches `"h"`, and where it is not built the
  test deliberately uses a fresh `ConfigBuilder::new()`. Impact: those ten
  tests keep passing under bootstrap pruning, but they also mean the repository
  itself relies on declaring an unreferenced definition and attaching it
  elsewhere in the same configuration. That is the pattern the runtime
  rejection must keep working, and it does: reachability is computed from the
  *resulting* attachment map, not from the definitions alone.

- Observation (P1, 2026-10-16): exactly one existing call site exercises the
  runtime path with a definition and no logger mutation,
  `runtime_mutation_tests.rs:223-226`. It is the regression test for a
  definition that stays reachable through *untouched* loggers, and it is the
  single most sensitive existing test to this change. Impact: it constrains the
  design to seeding reachability from the whole committed attachment map, which
  `plan_mutation()` does.

- Observation (2026-10-16, during EP-M1): `HTTPHandlerBuilder` had no
  `HandlerBuilder` variant or `From` impl, so runtime transactions could not
  declare HTTP handlers. The user authorized an exception to the plan's scope
  tolerance on 2026-10-23, allowing this gap to be closed by adding the
  existing builder to the config union and Python extractor; no method
  signature changes are required.

- Observation (2026-10-16, during EP-M1): extracting `LoggerMutationBuilder`
  into `runtime_mutation/logger_mutation.rs` to satisfy the 400-line limit
  initially broke the Python surface. Moving the struct into a child module
  removed `pyclass` from scope, made its Python-binding fields private to the
  child module, and made `ensure_valid` private to `runtime_mutation`. Impact:
  the extraction had to re-import `pyo3::prelude::*`, widen the Python-binding
  fields and the three methods to `pub(crate)`, and re-export the type with
  `pub use` rather than `pub(crate) use` so the public path stays
  `config::LoggerMutationBuilder`. A file split that merely moves code can
  still change an API; only a compile of every lane proves it did not.

- Observation: at 19:31 on 2026-10-16 the shared `build-limits` compile
  admission supervisor died machine-wide, in the middle of `make lint`'s
  `tracing-compat` clippy lane. Evidence:
  `build-limits: the supervisor refused: unhealthy` in
  `/tmp/lint-issue-434-...-after-mutation-3.out`, followed by
  `buildlimits-supervisor.service` failing with status 2 and
  `build-limits-status` reporting `Connection refused` when querying
  `/run/user/1000/buildlimits/supervisor.sock`. The unit refuses to restart
  itself, logging that start-up requires operator action because it cannot
  account for managed cache work, and instructing the operator to coordinate a
  quiet sccache stop, verify its empty cgroup, then restart admission. Impact:
  no Rust-compiling gate (`make lint` Rust lanes, `make typecheck`,
  `make test`) can produce a verdict until an operator restores admission. This
  is infrastructure, not a defect in this change: the log contains zero
  source-level compile errors before the refusal. Per the project instructions
  this must be reported rather than worked around, so `RUSTC_WRAPPER` and
  `RUSTDOC` were left untouched and no direct `rustc` call was made.

- Observation (2026-10-16, resolving the Clippy question): once admission
  recovered, the re-run compiled all five strict `-D warnings`
  `lint-rust-clippy` lanes clean, so Clippy is verified. The gate then failed
  one step later, on the Whitaker Dylint suite, with a real finding in the new
  code: `Module prune_test_support must start with an inner doc comment.`, at
  `--> src/config/test_utils.rs:21:5` (lines 5363 and 5378 of
  `/tmp/lint-issue-434-...-after-mutation-11.out`;
  `LINT_EXIT=2 DURATION=5386s`). The rule inspects the first item *inside* a
  module, so the outer `///` comment already on the `mod` item did not satisfy
  it; the doc block had to move into the body as `//!`. Impact: a module's doc
  comment placement is load-bearing for this repository's Dylint suite, and a
  doc comment that renders correctly in rustdoc can still fail the gate.

- Observation (2026-10-16, second occurrence of the supervisor fault): the same
  supervisor fault recurred during the re-run, this time refusing the final
  `lint-rust-clippy` lane after every earlier lane had compiled. Evidence:
  `build-limits: the supervisor refused: unhealthy`, then
  `error: could not compile femtologging_rs (lib)`, then
  `# Rust lint lane failed: none (exit 101)`, at lines 2769-2774 of
  `/tmp/lint-issue-434-...-after-mutation-9.out`; `build-limits-status` reports
  `UNHEALTHY` with
  `drain check failed; tokens retained: retained
  detached work: live sccache service may retain detached requests; visible
  compiler roots=0; operator accounting required`,
  and the unit is `deactivating (stop-sigterm)`. The required recovery action
  is to restore manager connectivity and account for detached sccache requests,
  which only an operator can do. Impact: the same as the first occurrence, and
  it sharpens the diagnosis. The full log contains no `-->` file:line pointer,
  no `clippy::` lint name, and no `warning:` line, and none of the
  change-surface files appear in any diagnostic, so there is no source-level
  evidence to act on. Note also that `lint-env-policy`'s seven lanes and
  `lint-rust-clippy`'s first lane (`none` for the policy run) all reported
  `the managed sccache server is unreachable; compiling without the cache`,
  which the project instructions explicitly classify as not a build failure.

- Observation (2026-10-16, correcting an over-claim): the seven green Rust
  lanes are *not* evidence that Clippy is clean. They belong to
  `lint-env-policy`, whose `ENV_POLICY_LINT_ARGS` are
  `-A clippy::all -D clippy::disallowed_methods` — the `all` group is allowed,
  so those lanes check the environment-access policy and not Clippy's
  diagnostics. The `-D warnings` lanes are `lint-rust-clippy`'s, and only its
  first lane (`none`) started before the refusal; it did not finish, and the
  remaining four (`python`, `log-compat`, `tracing-compat`, `all`) were never
  reached. `make lint`'s Clippy verdict was therefore **unknown**, not passing.
  Impact: the re-run had to complete `lint-rust-clippy` before this change
  could be considered to have passed the lint gate, and no earlier green lane
  could be cited in its place. That re-run is the `-11` log, and it did
  complete all five `-D warnings` lanes clean — so Clippy is now verified, and
  the over-claim was confined to the interim period before that run finished.

- Observation (2026-10-16, 22:53): admission is degraded but not refused. A
  socket-level probe shows `build-limits-supervisor` (pid 2867217) still
  listening on `/run/user/1000/buildlimits/supervisor.sock`, yet every
  `build-limits-status` query for roughly ten minutes returned
  `[Errno 11] Resource temporarily unavailable`, and `systemctl --user` could
  not reach the user manager at all
  (`Failed to retrieve unit state: Transport endpoint is not connected`).
  Twenty-seven `build-limits-rustc` processes under pid 2621545
  (`systemd --user`) are unreaped zombies, and the machine is running many
  concurrent `cargo`/`clippy` invocations from other agents. Impact: this is a
  different failure from the earlier refusals — admission is accepting work,
  not refusing it — and it presents as an unresponsive status query rather than
  an error, so a status probe alone must not be read as "not safe to build".
  The gates are therefore started despite the unreadable status, because the
  affected socket is only the ingress path; a genuine refusal would still be
  reported by the gates themselves.

- Observation (2026-10-16, 23:02-23:26; a third admission failure, distinct
  again): `make lint` had to be abandoned after 1143s. Evidence: the `-12` log
  shows the very first `lint-env-policy` lane's `clippy-driver` probe timing
  out rather than being refused —
  `[build-limits] clippy-driver: waiting for the build-limits supervisor (45s)`,
  then `waiting for a compile slot (0 ahead, 90s)` repeated to `270s`, then
  `build-limits: compile admission did not acknowledge the request before its
  deadline`,
  then `# Rust lint lane failed: none (exit 101)`. The tell is `0 ahead` with
  no slot ever granted: the queue was empty for *this* request while the
  supervisor was unresponsive to it. `build-limits-metrics` explains it — over
  the preceding hour the pool saw `queue_maximum: 194.0` with
  `queue_sample_mean: 151.98` against `capacity 4`, `refused: 114` with reason
  `launch_failed`, and wait times with `maximum 2383.297s` and `p99 1984.788s`.
  Impact: the failure is caused by demand from every agent on the machine, not
  by anything in this change, and it is transient — minutes later
  `build-limits-status` reported `capacity 4, held 0, healthy` and
  `no requests waiting`. The remedy is to retry the gate, not to alter the
  build. Note this is the first occurrence that is *not* the `unhealthy`
  refusal and not the unresponsive-status-query of the previous observation;
  all three modes share one root cause, a pool this small serving this many
  concurrent agents.

## Decision Log

- Decision: pause implementation when the scope tolerance is reached. The
  tracked diff contains 592 insertions and 225 deletions across 14 files; the
  untracked implementation and test files add 1,068 lines across 5 paths, for
  approximately 1,435 net added lines across the implementation. Including the
  separate 931-line plan artefact, the worktree has 20 changed paths. This
  exceeds the stated 14-file or roughly 900-line tolerance, so the next step is
  user direction on a reduced slice or explicit acceptance of the wider scope.
  Date/Author: 2026-10-23 / Codex.

- Decision: continue beyond the tolerance and add HTTP builder support. The
  user explicitly authorized a scope exception on 2026-10-23. The acceptance
  criteria require HTTP churn coverage, and adding `HTTPHandlerBuilder` to the
  existing `HandlerBuilder` union and Python extractor is a bounded additive
  change that preserves method signatures. Date/Author: 2026-10-23 / Codex.

- Decision: reject supplied definitions that the resulting attachment graph
  cannot reach, rather than accepting and silently dropping them. Rationale:
  silent dropping makes an identifier-typo bug look like success and hides a
  misconfiguration; rejecting matches the existing fail-fast treatment of
  unknown identifiers, and the `InvalidMutation` error already maps to
  `ValueError` in Python. Date/Author: 2026-10-16 / Claude Code.

- Decision: the reachability check runs before construction, so a rejected
  mutation never opens a file or spawns a worker. Rationale: the acceptance
  criteria call for no resource acquisition on the rejection path, and building
  first would spend the resource before the check could reject it. Date/Author:
  2026-10-16 / Claude Code.

- Decision: pruning keys on the committed per-logger attachment metadata, not
  on live logger objects. Rationale: the metadata is already the source of
  truth for identifier-level operations such as "remove handler `audit`", and
  reading live logger vectors would require the GIL and would not extend to the
  bootstrap path, which publishes before the plan is applied. Date/Author:
  2026-10-16 / Claude Code.

- Decision: apply the same rule to `ConfigBuilder.build_and_init()` while
  building and validating every declared definition. Rationale: one invariant
  for both publication paths keeps them from drifting apart, which is the
  failure mode this defect demonstrates. Date/Author: 2026-10-16 / Claude Code.

- Decision: reject a transaction that detaches an identifier and redefines it
  under the same identifier. Rationale: construction happens before the
  detaching mutation is known to succeed, and the objects would be published by
  a later reachability pass and then immediately pruned, so the transaction
  would be internally inconsistent. Multi-step workflows that legitimately
  detach and later redefine an identifier remain supported across two
  transactions. Date/Author: 2026-10-16 / Claude Code.

- Decision: leave the repository layering exactly as it is.
  Rationale: P2 confirmed that `config::reachability` is reachable from both
  the bootstrap module and the runtime mutation module, both of which are
  `#[cfg(feature = "python")]` and both of which already depend on
  `crate::manager`. Date/Author: 2026-10-16 / Claude Code.

## Outcomes & Retrospective

Implementation is complete and the deterministic evidence is in hand; the draft
pull request is the remaining deliverable.

What landed, by requirement:

- REQ-434-1 (reachable-only registries). `config/reachability.rs` computes the
  reachable identifier set from logger attachment states, and both publication
  paths prune against it: `assemble_commit()` for runtime mutation and
  `build_and_init()` for bootstrap. Pruning happens on the maps handed to
  `replace_runtime_state()`, so a committed registry is reachable-only by
  construction rather than by a later sweep.
- REQ-434-2 (reject before construction). `apply()` validates, snapshots,
  plans, and calls `reject_unreachable_definitions()` before `build_handlers()`
  and `build_filters()` run. The regression test asserts the target file does
  not exist after rejection, which is the observation that distinguishes
  "rejected up front" from "built then dropped".
- REQ-434-3 (release superseded objects). Pruning drops the registry's strong
  reference in the same commit. `runtime_lifecycle_tests.rs` observes release
  with `Arc::downgrade()` on the registry entry, and the gated-handler test
  pins the ordering: the object survives while a record is in flight and is
  released once the queue drains.
- REQ-434-4 (bootstrap parity). `build_and_init()` still builds and validates
  every declared definition — so a declaration error in an unattached
  definition still surfaces — and publishes only what the applied plans reach.
- REQ-434-5 (documentation). `docs/configuration-design.md` section 3.2 states
  the lifecycle rule, the absence of dormant definitions and of an unregister
  operation, the pre-construction rejection, and the release-and-drain caveat;
  the 3.2.4 ExecPlan carries a follow-up section recording the two behaviour
  changes that refine its original text.

Evidence: `cargo check --no-default-features --features python --all-targets`
passes with no warnings, and
`cargo test --no-default-features --features python --lib config::` reports 58
passed, 0 failed — including all 9 `reachability_tests`, 9
`runtime_pruning_tests`, and 5 `runtime_lifecycle_tests`. Logs:
`/tmp/check-issue-434.out`, `/tmp/test-issue-434.out`.

One deliberate deviation from the Coding Plan, recorded under
`Surprises & Discoveries`:

1. `runtime_mutation.rs` and `runtime_mutation_tests.rs` both exceeded the
   400-line project limit, so `LoggerMutationBuilder` and the pruning tests
   were split into their own modules. The split changed visibility and required
   explicit `pub(crate)` widening plus a `pub use` re-export to keep the public
   path unchanged.

Retrospective: the defect was cheap to describe and expensive to bound, because
"unreachable" has two different right answers depending on the caller.
`apply()` must *reject* an unreachable definition, since silently dropping a
caller's explicit declaration makes a typo look like success.
`build_and_init()` must *not* reject, because declaring a definition and
attaching it elsewhere is an established, tested pattern in this repository.
Writing out that asymmetry before editing — and finding the one existing test
(`replacing_shared_handler_id_updates_untouched_loggers`) that constrains how
reachability is seeded — was what kept the change small. The most useful
surprise was structural: a pure code move across a module boundary is not
API-neutral in Rust, and only compiling every lane proved the public surface
was intact.

## Conformance basis

The upstream artefact for this work is GitHub issue #434, "Prune unreferenced
runtime handler and filter definitions after mutation", reproduced in the task
brief. The repository has no Terms of Reference or technical design revision
numbered for this defect, and none is invented here. The design element this
plan changes is section 3 ("Runtime Reconfiguration") of
`docs/configuration-design.md`, which currently documents transactional apply
semantics and manager-owned runtime metadata without a lifecycle rule for
definitions. Roadmap item 3.2.4, delivered by
`docs/execplans/3-2-4-richer-runtime-handler-and-filter-mutation.md`, is the
feature this defect was found in; its Decision Log records the "shared
configuration identity" rule that this work extends.

Traced requirements:

```plaintext
REQ-434-1 published registries hold only reachable definitions      -> EP-M1, EP-M2 -> L1, T1..T9
REQ-434-2 unreachable supplied definitions rejected pre-construction -> EP-M1        -> L2, T1, T2, T6
REQ-434-3 superseded objects released after in-flight drain         -> EP-M1        -> L3, T7, T8
REQ-434-4 bootstrap publication follows the same rule               -> EP-M2        -> T5
REQ-434-5 lifecycle semantics documented                            -> EP-M4        -> D1, D2, D3
```

## Verification plan

### Invariants and lemmas

Invariant I1 (registry containment). After any successful `apply()` or
`build_and_init()`, every key of the published handler registry appears in the
handler identifier list of at least one entry of the published logger
attachment map, and likewise for filters.

Invariant I2 (definition legality). A definition supplied to `apply()` whose
identifier appears in no resulting attachment list is rejected, and no object
is constructed for any supplied definition, reachable or not, before the
rejection.

Invariant I3 (single live object per identifier). At most one published object
exists per identifier, and a replaced identifier's previous object is not
reachable from the registries or from any impacted logger once `apply()` has
returned.

Invariant I4 (deferred release). An object removed from the registries stays
alive while any in-flight record or live logger still references it, and is
released once those references end.

Lemma L1 (pruning preserves resolvability). Because `apply_commit()` resolves
each impacted logger's attachments from the pruned registries, and pruning
removes exactly those keys that no attachment list mentions, the resolution
step cannot fail for a reachable mutation. This is what keeps I1 from breaking
live logging.

Lemma L2 (bootstrap plan coverage). Every identifier an applied plan attaches
is present in that plan's identifier list, so pruning against the plan union
cannot remove an object an applied logger holds.

Lemma L3 (error precedence). `validate()` runs first, then planning
(no-runtime-metadata and unknown-identifier errors), then the reachability
rejection, then construction. A mutation that references an unknown identifier
*and* supplies an unreachable definition therefore reports `UnknownIds`.

### Axioms

- `std::sync::Arc` releases its payload when the last strong reference is
  dropped, and `Weak::upgrade()` returns `None` afterwards.
- `parking_lot::RwLock` gives mutual exclusion, so `snapshot_runtime_state()`
  returns a consistent clone of the runtime state.
- `FemtoFileHandler`, `FemtoSocketHandler`, and `FemtoHTTPHandler` construct
  their workers eagerly but connect or write lazily, so constructing a handler
  against an unused address does not block.
- The existing bounded close timeouts (1s for the file handler) are a sufficient
  upper bound for the release observed after a drain.

### Obligations

- Obligation: I1 and I2 hold for a mutation that declares a definition no logger
  attaches. Method: parameterized Rust test plus a Python test. Rationale: the
  property is a discrete accept/reject statement with a side-effect observation
  (no file created), which a named example expresses exactly; no generator adds
  value. Domain: stream, file, socket, and HTTP definitions; handler and filter
  kinds. Artefact: `rust_extension/src/config/runtime_mutation_tests.rs`,
  `tests/test_runtime_reconfiguration.py`. Evidence:
  `cargo test --no-default-features --features python runtime_mutation` fails
  on the new test before EP-M1 and passes after;
  `uv run pytest tests/test_runtime_reconfiguration.py` likewise. Non-vacuity:
  the same test body attaches the definition in a companion case and asserts
  success, so the rejection path is exercised against a
  simultaneously-satisfiable acceptance path; a mutation that ignores
  reachability fails the companion case with "expected ValueError" and a
  mutation that rejects everything fails the attach case.

- Obligation: I1 holds after N rounds of churn with fresh unused identifiers.
  Method: bounded parameterized test (N = 32) over the three resource-owning
  handler kinds. Rationale: the defect is cumulative, so a single example
  cannot discharge it; 32 rounds is enough to distinguish "constant" from
  "linear in N" without a generator. Domain: file handlers on a
  `tempfile::tempdir()` path, socket handlers built without a listener, HTTP
  handlers built against an unused loopback address. Artefact:
  `rust_extension/src/config/runtime_lifecycle_tests.rs`. Evidence: registry
  length asserted constant and equal across all rounds; the temporary directory
  asserted empty afterwards. Non-vacuity: the first round asserts the rejection
  is the *new* error and not a pre-existing one, and a companion round uses a
  *reachable* definition of the same kind, which must be accepted and
  published. This proves the test can distinguish the two outcomes.

- Obligation: I3 and I4 hold for a same-identifier replacement and for the
  removal of a last reference. Method: named Rust tests using
  `Arc::downgrade()` on the published registry entry, with a bounded wait after
  draining the logger queue. Rationale: object lifetime is not observable
  through behaviour alone, and process-wide thread or descriptor counts would
  be flaky under a shared manager; a weak reference is the precise observation.
  Domain: `FemtoFileHandler` replacement, last-reference removal, and a gated
  in-flight record. Artefact:
  `rust_extension/src/config/runtime_lifecycle_tests.rs`. Evidence:
  `Weak::upgrade()` returns `Some` while the in-flight record holds the object
  and `None` after the gate opens and the record drains. Non-vacuity: the
  in-flight assertion and the post-drain assertion are opposites on the same
  weak reference, so neither a never-dropping nor an eagerly-dropping
  implementation can satisfy both; the gate is a channel the test observes
  before mutating, so "in flight" is established, not assumed.

- Obligation: L1, L2, and L3 hold, and the pre-existing suite still passes.
  Method: the repository's own gates plus the new focused tests. Rationale:
  these are regression obligations against existing behaviour. Domain: whole
  crate, all feature lanes used by `make test`. Artefact:
  `rust_extension/src/config/config_tests.rs`, `runtime_mutation_tests.rs`, and
  the Python suites. Evidence: `make test`, `make lint`, `make typecheck`,
  `make check-fmt`, `make markdownlint`, and `make nixie` pass. Non-vacuity:
  the pre-existing `replacing_shared_handler_id_updates_untouched_loggers` test
  must keep passing, and it fails if pruning drops a shared handler that a
  surviving logger still references.

## Plan of work

### Stage A: prerequisites (no production changes)

P1. Search `tests/` and `rust_extension/src/` for configurations that declare a
handler or filter no logger attaches, to size the blast radius of the new
rejection. Record findings in `Surprises & Discoveries` or in the Decision Log.

P2. Confirm that a new `rust_extension/src/config/reachability.rs`, declared in
`rust_extension/src/config/mod.rs` under `#[cfg(feature = "python")]`, can be
used from both `config::build` and `config::runtime_mutation`. This is a
read-only check against the module declarations and existing imports.

P3. Capture red evidence: add the `unused_definition_rejected_before_build`
test to `rust_extension/src/config/runtime_mutation_tests.rs` and run it. It
must fail with a "should have been rejected" panic because `apply()` currently
succeeds, and the target file must already exist at that point, proving the
resource is acquired today.

### Stage B: the reachability helper and the runtime path

Add `rust_extension/src/config/reachability.rs`. It owns:

- `ReachableIds`, holding two `BTreeSet<String>` values, with constructors
  `from_logger_states()`, `from_attachment_ids()`, and
  `from_logger_states_and_handler_filters()`.
- `prune_handlers()` and `prune_filters()`, which take an owned registry map by
  mutable reference and retain only reachable keys.
- `unreachable_definition_ids()`, which returns the sorted identifiers present
  in the supplied set but absent from the reachable set, and
  `unreachable_definition_error()`, which turns those identifiers into a
  `ConfigError::InvalidMutation` naming handler and filter identifiers
  separately.

Nothing in this module touches Python, the manager, or a registry value type;
each function is generic over the value so it serves both
`Arc<dyn FemtoHandlerTrait>` and `Arc<dyn FemtoFilter>` maps.

Add `KnownIds` to `runtime_mutation/validation.rs` and change
`resolve_attachment_ids()` to take it instead of the two registry maps, so
validation can accept identifiers that are supplied-but-not-yet-built while
still rejecting genuinely unknown ones.

Reorder `RuntimeConfigBuilder::apply()` into these steps: call `validate()`;
enter `Python::attach`, take the configuration transaction lock, and snapshot
the runtime state; plan logger attachment mutations against the pre-mutation
registries plus supplied identifiers; validate filters named by supplied
handlers; reject unreachable supplied definitions; build the supplied
definitions; assemble the commit (merge, prune, resolve impacted loggers); and
apply it. The transaction lock does not hold the manager lock during
construction.

Handler-level filter IDs are a second graph edge. A filter is reachable through
one of those IDs only while its handler remains attached to a logger. Merge the
old and newly supplied handler-filter maps for planning, then prune the map to
the retained handlers before resolving their filters.

Split `prepare_commit()` into planning and assembly so the planning half is
pure and the assembly half owns the merge and prune. Keep `BuiltRegistries` and
`RuntimeCommit` as the seam between them.

### Stage C: the bootstrap path

In `build_and_init()`, replace the two `built_handlers.clone()` /
`built_filters.clone()` arguments with moved maps that have been pruned against
the union of the applied plans' identifier lists. Build and validate every
declared definition first, exactly as now.

### Stage D: regression coverage

Extend `rust_extension/src/config/runtime_mutation_tests.rs` with identifier
level semantics: unused handler rejection, unused filter rejection, empty
mutation, define-and-attach, last-reference pruning, shared-handler survival,
bootstrap pruning, and unknown-identifier precedence.

Add `rust_extension/src/config/reachability_tests.rs` for the helper in
isolation, and `rust_extension/src/config/runtime_lifecycle_tests.rs` for the
lifetime and churn properties over file, socket, and HTTP handlers. HTTP
builder support is added to the existing config builder union so these tests
exercise the same transaction path as the other handler types.

`tests/test_runtime_reconfiguration.py` gains the Python-level coverage,
including the churn test that replaces one reachable file handler id across
`CHURN_ROUNDS` rounds and asserts the live handler count stays bounded. Because
each round first appends the id and then redefines it without re-attaching, the
test also pins the rule that a *reachable* id may be redefined in a transaction
that names no logger: the id is still reachable through the logger that already
attaches it, so the redefinition is accepted and swaps in place.

`tests/features/runtime_reconfiguration.feature` already carries the runtime
mutation failure modes (`Failed mutation preserves the prior state` for the
`KeyError` path), so the `ValueError` rejection belongs there too rather than
only at unit level. The scenario asserts the registry is unchanged after the
rejection, which is the user-visible half of the rule: a caller who mistypes an
identifier must not be left believing the definition was published.

### Stage E: documentation and pull request

Document the lifecycle rule in `docs/configuration-design.md` section 3,
cross-reference it from
`docs/execplans/3-2-4-richer-runtime-handler-and-filter-mutation.md`, update the
`apply()` Rustdoc and the Python stub, then open the draft pull request.

## Milestones and plateaus

- EP-M1 (Stage B). Outcome: the runtime mutation path plans, rejects, builds,
  prunes, and publishes; no unreachable definition survives a successful
  mutation, and no unreachable definition is constructed for a rejected one.
  Requirements and gaps: REQ-434-1, REQ-434-2, REQ-434-3 advanced. Acceptance
  evidence: `runtime_mutation_tests.rs` gains
  `unused_definition_rejected_before_build` (red before, green after) and the
  existing runtime mutation tests still pass. Recovery: revert to the pre-change
  `apply()` ordering; the helper module is additive and can stay. Remaining
  gaps: bootstrap publication, lifetime tests, documentation. Conformance
  check: requirements satisfied for the runtime path; design still followed
  (`manager.rs` untouched); upstream assumptions still valid (the attachment
  metadata remains the identifier source of truth); no unapproved public
  interface or dependency change (the new rejection uses the existing
  `InvalidMutation` variant); no persisted-format or trust-boundary change;
  trace links current. Compatibility decision: none. This is an unreleased
  internal control plane and the rejection is a deliberate behaviour change
  recorded in the Decision Log.

- EP-M2 (Stage C). Outcome: bootstrap publication obeys the same containment
  invariant without changing which loggers or handlers it configures.
  Requirements and gaps: REQ-434-4 discharged. Acceptance evidence: the new
  bootstrap-pruning test passes and `config_tests.rs` is unchanged and green.
  Recovery: revert the `build_and_init()` hunk; the runtime path is unaffected.
  Remaining gaps: lifetime tests, documentation. Conformance check:
  requirements satisfied; design still followed; upstream assumptions still
  valid; no unapproved interface change; trace links current. Compatibility
  decision: none.

- EP-M3 (Stage D). Outcome: the property is guarded by tests that can fail.
  Requirements and gaps: REQ-434-1, REQ-434-2, REQ-434-3, REQ-434-4 verified.
  Acceptance evidence: `cargo test --no-default-features --features python`
  green including the new modules;
  `uv run pytest tests/test_runtime_reconfiguration.py` green. Recovery: tests
  are additive; drop the new modules to roll back. Remaining gaps:
  documentation and the pull request. Conformance check: requirements
  satisfied; design still followed; upstream assumptions still valid; no
  unapproved interface change; trace links current. Compatibility decision:
  none.

- EP-M4 (Stage E). Outcome: the lifecycle rule is documented, the branch is
  pushed, and a draft pull request is open with all deterministic gates green
  before the CodeRabbit review is requested. Requirements and gaps: REQ-434-5
  discharged; all requirements closed. Acceptance evidence: `make markdownlint`,
  `make nixie`, and the other gates pass; the pull request body states
  `Closes #434` and links the session. Recovery: revert the documentation
  commit; the code is unaffected. Remaining gaps: none. Conformance check:
  requirements satisfied; design still followed; upstream assumptions still
  valid; no unapproved interface change; trace links current. Compatibility
  decision: none.

## Concrete steps

All commands run from the worktree root.

Prerequisite P3, red evidence:

```sh
set -o pipefail && cargo test --manifest-path rust_extension/Cargo.toml \
  --no-default-features --features python \
  runtime_mutation_tests::unused_definition_rejected_before_build 2>&1 | \
  tee /tmp/test-issue-434-prune-unreferenced-runtime-handler-and-filter-definitions-after-mutation.out
```

Expected: the test fails because `apply()` returned `Ok(())`, and the assertion
that the handler's file was never created also fails.

Green evidence for the same test, after EP-M1, is the same command exiting 0.

Full gates are delegated to the `scrutineer` subagent at each milestone, using
`make fmt`/`make check-fmt`, `make lint`, `make typecheck`, `make test`,
`make markdownlint`, and `make nixie`, with per-gate logs under `/tmp`.

## Validation and acceptance

Behavioural acceptance, exercised from Python:

1. `RuntimeConfigBuilder().with_handler("orphan", FileHandlerBuilder(path))`
   with no logger mutation raises `ValueError` naming `orphan`, and `path` does
   not exist afterwards.
2. Repeating step 1 with 32 fresh identifiers leaves the runtime registry length
   unchanged and the temporary directory empty.
3. Declaring the same handler and attaching it to a logger in one transaction
   succeeds, and the file receives records.
4. Removing the last reference to an attached handler removes it from the
   registry, and the handler object is released once the logger queue drains.

Quality criteria:

- Tests: `make test` passes; new Rust modules and the new Python tests are part
  of it.
- Verification: invariants I1-I4 and lemmas L1-L3 discharged as described in
  `Verification plan`.
- Lint and typecheck: `make lint`, `make check-fmt`, and `make typecheck` pass
  with no suppression added.
- Documentation: `make markdownlint` and `make nixie` pass.

## Idempotence and recovery

Every step is additive or a local reordering. The failing-test stage is
repeatable without side effects. If a `Weak`-based test proves flaky under
load, replace the fixed sleep with a poll loop bounded by the existing handler
close timeout rather than relaxing the assertion. If a gate fails, re-run only
that gate after applying a fix, and read the captured log under `/tmp` rather
than re-running it blind.

## Artefacts and notes

Evidence transcripts are appended here as milestones complete.

## Interfaces and dependencies

In `rust_extension/src/config/reachability.rs`:

```rust
pub(crate) struct ReachableIds {
    pub(crate) handlers: BTreeSet<String>,
    pub(crate) filters: BTreeSet<String>,
}

impl ReachableIds {
    pub(crate) fn from_logger_states<'a>(
        states: impl IntoIterator<Item = &'a LoggerAttachmentState>,
    ) -> Self;

    pub(crate) fn from_attachment_ids<'a>(
        attachments: impl IntoIterator<Item = (&'a [String], &'a [String])>,
    ) -> Self;

    pub(crate) fn from_logger_states_and_handler_filters<'a>(
        states: impl IntoIterator<Item = &'a LoggerAttachmentState>,
        handler_filter_ids: &BTreeMap<String, Vec<String>>,
    ) -> Self;
}

pub(crate) fn prune_handlers<T>(registry: &mut BTreeMap<String, T>, reachable: &ReachableIds);
pub(crate) fn prune_filters<T>(registry: &mut BTreeMap<String, T>, reachable: &ReachableIds);

pub(crate) fn unreachable_definition_ids(
    supplied: &ReachableIds,
    reachable: &ReachableIds,
) -> Option<(Vec<String>, Vec<String>)>;

pub(crate) fn unreachable_definition_error(
    handlers: &[String],
    filters: &[String],
) -> ConfigError;
```

In `rust_extension/src/config/runtime_mutation/validation.rs`:

```rust
pub(crate) struct KnownIds {
    pub(crate) handlers: BTreeSet<String>,
    pub(crate) filters: BTreeSet<String>,
}

impl KnownIds {
    pub(crate) fn new(
        handlers: impl IntoIterator<Item = String>,
        filters: impl IntoIterator<Item = String>,
    ) -> Self;
}

pub(crate) fn resolve_attachment_ids(
    state: &LoggerAttachmentState,
    known: &KnownIds,
) -> Result<(), ConfigError>;
```

No new external dependencies are introduced.

## Revision note

- 2026-10-16: initial draft. No revision to earlier content.
- 2026-10-23: paused after the resumed worktree exceeded the scope tolerance.
  The user then explicitly authorized an exception and continuation; HTTP
  builder support is being added to satisfy the original acceptance criteria.
- 2026-10-24: CodeRabbit reported a lost-update race because concurrent runtime
  mutations could snapshot the same registries and later replace one another's
  commits. Accepted the finding: serialize snapshot, planning, construction,
  and commit with a configuration-scoped transaction mutex, and take the same
  lock while bootstrap publishes its logger state. The manager's `RwLock`
  remains short-lived and `manager.rs` stays functionally unchanged. Added a
  Rust concurrent-apply regression test.
- 2026-10-24: fixed CodeRabbit's low-severity test finding by making the file
  churn test require exactly `baseline + 1` attached handlers instead of
  deriving the expected count from the observed result.
- 2026-10-25: addressed the follow-up CodeRabbit findings, including the
  duplicated stream-target mapping in the BDD steps. The final full gate run
  passed all six deterministic gates, and a CodeRabbit retry reviewed all 24
  paths with zero findings after the required cooldown.
- 2026-10-25: `origin/main` advanced to `d234990f` while final validation was
  running. Rebased the issue commit onto that target; the upstream contextual
  handler-filter graph required carrying each attached handler's filter IDs
  into reachability and pruning. Full gates and review must be repeated for
  this candidate because the rebase changed its source tree.
