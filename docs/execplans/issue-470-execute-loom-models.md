# Execute the Loom models in the scheduled heavy lane

This ExecPlan is a living document. The sections `Constraints`,
`Tolerances (exception triggers)`, `Risks`, `Progress`,
`Surprises & discoveries`, `Decision log`, `Outcomes & retrospective`,
`Conformance basis` and `Verification plan` must be kept up to date as work
proceeds.

Status: DRAFT — awaiting approval. No production code changes until the plan is
approved.

Related: [Issue #470](https://github.com/leynos/femtologging/issues/470).

## Purpose / big picture

Loom is a model checker for concurrent Rust. Given a block of code, it runs
that block many times over, choosing a different thread interleaving each time,
until it has explored every interleaving the memory model permits within its
bounds. It can only do this for operations it controls, so code under a Loom
model must spawn threads with `loom::thread`, share values through
`loom::sync::Arc`, and lock with `loom::sync::Mutex`. An operation performed
with the standard library's primitives is invisible to Loom, and touching a
Loom value from a thread Loom did not create aborts the whole process.

This repository contains six Loom model functions and runs none of them. They
live in `rust_extension/tests/heavy/`, in three modules gated behind
`#[cfg(loom)]`:

- `loom_push.rs` defines `loom_stream_push_delivery`, which has two threads
  push a record each through one `FemtoStreamHandler` and asserts both records
  reach the shared buffer;
- `loom_file_flush.rs` defines `loom_file_handler_flush_concurrent`, which has
  five threads each write a record through one `FemtoFileHandler` and call
  `flush`, then asserts the file holds five lines;
- `loom_topologies.rs` defines `loom_single_logger_multi_handlers`,
  `loom_shared_handler_multi_loggers`,
  `loom_multiple_loggers_multiple_handlers` and
  `loom_concurrent_handler_addition`, which assert that records are routed to
  every attached handler exactly once under concurrent logging and concurrent
  handler attachment.

The scheduled `heavy-tests` workflow compiles them and stops. Its Loom step
passes `--no-run`, and its four test steps do not set `--cfg loom` at all, so
the model modules are not even compiled in those steps. A green scheduled run
therefore establishes that the models type-check, and nothing about the
behaviour they describe.

The reason is recorded in `rust_extension/tests/heavy/main.rs`:
`FemtoStreamHandler` and `FemtoFileHandler` each start their worker with
`std::thread::spawn`. That worker then writes into a Loom-instrumented buffer
from a thread Loom did not create, and Loom aborts the process with "cannot
access Loom execution state from outside a Loom model", taking the whole heavy
binary down. Removing `--no-run` today would replace a misleading green with a
crash.

After this work, a maintainer can run one command and watch named models
execute, and the daily schedule does the same and fails loudly if it executes
none:

```shell
RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 cargo test \
  --manifest-path rust_extension/Cargo.toml \
  --no-default-features --test heavy loom_ -- --include-ignored
```

The expected tail of that command is a named, non-zero result:

```plaintext
test loom_file_handler_flush_concurrent ... ok
test loom_concurrent_handler_addition ... ok
test loom_multiple_loggers_multiple_handlers ... ok
test loom_shared_handler_multi_loggers ... ok
test loom_single_logger_multi_handlers ... ok
test loom_stream_push_delivery ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out
```

## Orientation for a newcomer

Four areas of the repository are involved, and nothing outside them is touched.

`rust_extension/src/stream_handler.rs` holds `FemtoStreamHandler`. Its private
constructor `with_config` creates a bounded channel for records, a second
bounded channel for the worker's shutdown acknowledgement, and spawns the
worker at line 276. Its `close` waits on the acknowledgement channel with
`recv_timeout`.

`rust_extension/src/handlers/file/worker.rs` holds the equivalent for
`FemtoFileHandler`, spawning at line 318.

`rust_extension/tests/heavy/` holds the models described above, plus
`rust_extension/tests/test_utils/shared_buffer.rs`, which already provides two
parallel implementations of a shared output buffer, one on the standard library
and one on Loom, selected by the test rather than by configuration.

`.github/workflows/heavy-tests.yml` holds the scheduled lane. It runs at 17:00
UTC daily, supports manual dispatch, and its "Run formatters and tests" step
loops over four feature sets calling `cargo test ... -- --ignored`.

## The central design problem

Loom can only see operations performed through its own primitives. The
handlers' concurrency is currently built from three libraries, none of which
Loom instruments:

- threads come from `std::thread`;
- record and acknowledgement channels come from `crossbeam_channel::bounded`;
- interior mutability comes from `parking_lot::Mutex`.

Replacing only the spawn, as the existing module documentation suggests, is not
sufficient. A Loom-created thread that then blocks on a `crossbeam_channel`
receive blocks a thread Loom believes it controls, on a primitive Loom cannot
schedule. Loom's executor cannot advance, and the model deadlocks rather than
explores. The seam must therefore cover the thread, the channel and the mutex
together, or the models will fail for a second reason immediately after the
first is fixed.

A fourth problem has no mechanical answer. Loom has no clock. It explores
interleavings, not durations, and provides no analogue of `recv_timeout`. Both
handlers use a timeout on the shutdown acknowledgement, and
`FemtoStreamHandler` also exposes `flush_timeout`. Under the model these must
become unbounded waits, because "the worker did not answer within a second" is
not a statement about an interleaving, and a model that can time out reports a
spurious failure on a schedule Loom deliberately chose. Making them unbounded
under `cfg(loom)` turns a timeout into a liveness obligation: if the worker can
fail to answer on some interleaving, the model hangs rather than fails. That is
the correct outcome, because such an interleaving is a real defect, but it
means the lane needs its own wall-clock job timeout as the outer bound.

## Constraints

- The production threading and performance characteristics are unchanged in
  every ordinary build. The seam resolves to `std::thread`, `crossbeam_channel`
  and `parking_lot` unless `--cfg loom` is set, and no ordinary build gains an
  indirection that the compiler does not remove.
- No public API changes. `FemtoStreamHandler`, `FemtoFileHandler`,
  `FemtoLogger` and the handler trait keep their current signatures and paths.
- The existing six models are repaired and kept. They are not replaced by
  freshly written models that avoid the problem, and their assertions are not
  weakened to make them pass.
- A model is never made to pass by substituting an uninstrumented buffer for a
  Loom-instrumented one. If a value must leave the model, the boundary is named
  and justified in `Decision log`.
- Scheduled runs stay on GitHub-hosted runners, per the estate placement rule:
  only pull-request, push and tag lanes move to Ubicloud.
- Every commit leaves the repository green on `make test`, `make lint`,
  `make check-fmt`, `make typecheck`, `make spelling`, `make markdownlint` and
  `make nixie`.

## Tolerances (exception triggers)

- Scope: if the seam requires editing more than fifteen files under
  `rust_extension/src/`, stop and escalate.
- Interface: if any public signature must change, stop and escalate.
- Dependencies: if a new runtime dependency is required, stop and escalate. A
  new dev-dependency used only under `cfg(loom)` is within tolerance and is
  recorded in `Decision log`.
- Model runtime: if any single model exceeds ten minutes at the chosen
  preemption bound on a quiet host, stop and escalate rather than lowering the
  bound silently.
- Defects found: if repairing the seam exposes a genuine concurrency defect in
  a handler, stop and escalate before fixing it. A defect in production
  synchronization is a separate change with its own review, not a step in this
  one.
- Iterations: if a model still aborts after three attempts at one milestone,
  stop and escalate with the abort message and the interleaving Loom reports.

## Risks

- Risk: the seam is invasive enough that reviewers reject it as an
  architectural change made for a test.
  - Severity: high. Likelihood: medium.
  - Mitigation: milestone two delivers the seam alone, with the ordinary build
    provably unchanged, and is reviewable on its own. The evidence is that
    `cargo expand` or a disassembly shows no added indirection in a release
    build, and that every existing test passes unchanged.
- Risk: a repaired model exposes a real defect in a handler.
  - Severity: medium. Likelihood: medium. This is a success of the work, not a
    failure, but it stops this plan.
  - Mitigation: the tolerance above. Record the model, the interleaving and the
    failing assertion, file an issue, and escalate.
- Risk: the models are too large for Loom to explore in reasonable time. Five
  threads in `loom_file_handler_flush_concurrent`, each performing a write and
  a flush, is a large state space, and each handler worker adds a thread Loom
  must schedule.
  - Severity: medium. Likelihood: high.
  - Mitigation: milestone four measures it first and reduces the model's thread
    count with the reduction recorded and justified, rather than lowering the
    preemption bound, which would weaken every model at once.
- Risk: `loom_file_handler_flush_concurrent` performs real file input and
  output through `tempfile::NamedTempFile` and `std::fs::read_to_string`. File
  operations are not instrumented, and Loom re-runs the model body many times,
  so each run creates a new temporary file.
  - Severity: medium. Likelihood: high.
  - Mitigation: milestone four decides between keeping the real file with the
    external-effect assumption recorded, and moving the model onto a shared
    in-memory writer while a separate ordinary integration test keeps the real
    file behaviour. Either way the choice is recorded in `Decision log`.
- Risk: the scheduled lane reports green while executing nothing, which is the
  present defect reappearing in a new form.
  - Severity: high. Likelihood: medium.
  - Mitigation: milestone five's executed-count assertion, proved by mutation
    in both directions.

## Milestones

Each milestone ends in a state the repository can be left in.

### EP-M1: make the absence visible

Add the execution command to the scheduled workflow and to the developers'
guide, and add a workflow contract test asserting that the Loom step runs the
models rather than compiling them. Do not change any production code.

End state: the contract test passes, and a manual dispatch of `heavy-tests`
fails at the Loom step with the documented abort, which is the honest current
state. The workflow's other steps are unaffected, so the lane still reports on
everything it reported on before.

Acceptance: the manual run's link and its abort message are recorded here. The
contract test is proved in both directions: restoring `--no-run` fails it, and
dropping `--cfg loom` from the command fails it.

Recovery: the milestone is a workflow and test change only, revertible in one
commit.

Remaining gaps: every model still aborts.

This milestone is the red stage of Red-Green-Refactor for the whole plan. It is
expected to leave the scheduled lane red, which is why it is discussed with the
user before merging: a knowingly red schedule is a decision, not an accident,
and the pull request says so.

### EP-M2: the concurrency seam

Introduce one private module, `rust_extension/src/sync.rs`, exporting the
thread, channel and mutex operations the handlers use. Under `cfg(loom)` it
re-exports Loom's primitives; otherwise it re-exports `std::thread`,
`crossbeam_channel` and `parking_lot`. The shape is the one
`rust_extension/tests/test_utils/shared_buffer.rs` already uses for buffers, so
it is a pattern the repository has rather than a new idea.

The module names every operation the handlers need, including the bounded
channel constructor, the blocking and non-blocking sends and receives, the
timed receive, and the thread spawn and join. The timed receive is the one
operation whose two implementations differ in meaning rather than in spelling:
under Loom it waits without a bound, for the reason given above, and the
module's documentation says so at the definition.

End state: the module exists, is used by nothing yet, and is covered by unit
tests that exercise both configurations.

Acceptance: `cargo test` passes in the ordinary configuration and under
`--cfg loom`. `make lint` passes in both.

Recovery: an unused module, removable in one commit.

Remaining gaps: the handlers still spawn directly.

### EP-M3: the stream handler onto the seam

Move `FemtoStreamHandler` onto the seam, and run the three models that use it:
`loom_stream_push_delivery` and the four in `loom_topologies.rs`.

End state: those five models execute and pass. `loom_stream_push_delivery`
loses its `#[ignore]` reason, which no longer describes anything.

Acceptance: the command in "Purpose" run with a filter selecting those models
reports five passed and zero failed, with the transcript recorded here. Every
existing stream-handler test passes unchanged, which is what shows the ordinary
path was not altered.

Non-vacuity: a deliberate defect is introduced into the worker's record
handling, run through the model, and the failing model and assertion are
recorded. The defect is then reverted. Without this, five passing models are
consistent with five models that explore nothing.

Recovery: the seam is a type alias change; reverting the handler's use of it
restores the previous state.

Remaining gaps: the file handler.

### EP-M4: the file handler onto the seam

Move `FemtoFileHandler` onto the seam and run
`loom_file_handler_flush_concurrent`, having first decided the file-effect
question named in `Risks` and recorded the decision.

End state: the sixth model executes and passes, or its thread count is reduced
with the reduction justified in `Decision log` and the ordinary integration
test that covers the fuller case named beside it.

Acceptance: the transcript, and a measured duration for the model at the chosen
bound on a quiet host. Every existing file-handler test passes unchanged.

Non-vacuity: as for EP-M3, a deliberate defect in the flush acknowledgement
path, the failing model recorded, then reverted.

Recovery: as for EP-M3.

Remaining gaps: the lane does not yet prove it executed anything.

### EP-M5: the lane proves it ran

Harden the scheduled lane. It asserts a non-zero executed model count and the
presence of each expected model by name, distinguishes discovery from
compilation from execution in its summary, sets an explicit preemption bound
and job timeout, caches the Loom build under a key that includes the toolchain,
the lockfile, the feature set and the flags, and keeps that cache separate from
the ordinary build's.

End state: a run that executes no models fails, and says so in those words.

Acceptance: proved by mutation in both directions. Restoring `--no-run` fails
the lane at the count assertion rather than passing green; filtering the model
selection down to a name that matches nothing fails it for the same reason; and
the unmutated lane passes. A contract test asserts the command, not merely that
the string "loom" appears in the file.

Recovery: workflow-only, revertible in one commit.

Remaining gaps: documentation.

### EP-M6: say what is now true

Update `rust_extension/tests/heavy/main.rs`, `docs/developers-guide.md` and
`docs/roadmap.md` so that each distinguishes compiled coverage from executed
coverage, describes the seam, and states the bounded nature of the assurance:
Loom explores every interleaving within its preemption bound, which is a strong
statement about a bounded space and not a proof for all executions.

End state: no document claims the models are merely compile-checked, and none
claims Loom proves the handlers correct.

Acceptance: `make markdownlint`, `make nixie` and `make spelling` pass, and the
guide's Loom section names the command in "Purpose" verbatim.

## Verification plan

The obligations below are about repository-owned logic. Loom's own correctness
is treated as an axiom.

Invariant one, delivery. Every record accepted by a handler reaches that
handler's writer exactly once, under every interleaving Loom explores within
its bound. Method: the existing models, repaired. Artefact:
`loom_stream_push_delivery` and `loom_file_handler_flush_concurrent`. Evidence:
named passing results. Discharge: EP-M3 and EP-M4. Non-vacuity: the seeded
defects named in those milestones, each rejected by a named model for the
intended reason.

Invariant two, routing. A record logged through a `FemtoLogger` reaches every
attached handler exactly once and no other handler, including while a handler
is being attached concurrently. Method: the four topology models. Evidence: as
above. Discharge: EP-M3. Non-vacuity: a seeded defect that delivers to the
first handler only, rejected by `loom_single_logger_multi_handlers`.

Invariant three, the ordinary build is unchanged. The seam introduces no
runtime indirection and no behavioural change outside `cfg(loom)`. Method: the
existing test suite, unchanged, plus an inspection of the generated code for
one representative call. Evidence: a green `make test` with no test edited in
the same commit as the seam, and the inspection recorded. Discharge: EP-M2 and
EP-M3. Non-vacuity: this obligation is discharged by tests that already exist
and already pass, so it is proved by the seam's mutation instead: making the
ordinary configuration select Loom's primitives must fail the ordinary suite.

Invariant four, the lane cannot report success without executing models.
Method: a mutation-proved workflow contract and an executed-count assertion.
Evidence: the two mutations named in EP-M5. Discharge: EP-M5. Non-vacuity: the
restored `--no-run` mutation is precisely the present state of the repository,
so the assertion is proved against a real configuration rather than a
hypothetical one.

Lemma, needed to connect invariants one and two to the models: a handler worker
started through the seam under `cfg(loom)` runs inside the Loom model, and
every synchronization it performs is an operation Loom schedules. Method:
direct, by inspection of the seam plus the absence of the abort message, which
is Loom's own report that the lemma is false. Evidence: the models run to
completion rather than aborting. This is why EP-M1 is worth doing first: it
establishes that the abort is what currently happens, so its later absence
means something.

Axioms. Loom explores every interleaving within its configured preemption bound.
`crossbeam_channel` and `parking_lot` provide the ordering guarantees their
documentation states, in the ordinary configuration where Loom does not observe
them. File writes through `std::fs` are outside the model, which is the
residual gap recorded against EP-M4. Assurance is bounded by
`LOOM_MAX_PREEMPTIONS`; the plan does not claim a proof for unbounded
executions, and EP-M6 says so in the guide.

## Conformance basis

The upstream artefact is issue #470, whose "Required work" and "Acceptance"
sections are the requirement set. There is no separate terms-of-reference or
technical-design document for this work, and none is invented here.

Trace links:

- #470 "Repair the production/model boundary" maps to EP-M2, EP-M3 and EP-M4,
  and to invariants one, two and three.
- #470 "Execute, observe and protect the scheduled lane" maps to EP-M1 and
  EP-M5, and to invariant four.
- #470 "Acceptance" items one and three map to EP-M3 and EP-M4 evidence; item
  two, a successful scheduled run on the default branch, is recorded after
  merge and before the issue closes; item four maps to EP-M6.

`docs/adr-004-batching-optimizations-in-consumer-threads.md` describes the
worker model the seam must preserve and is read before EP-M2.

## Surprises & discoveries

- (2026-09-16, before implementation) The command the issue proposes as a
  starting point selects the wrong tests. `cargo test ... -- --ignored` runs
  only tests marked `#[ignore]`, and of the six models only
  `loom_stream_push_delivery` carries that attribute. The other five would be
  filtered out and the lane would report one test where it expects six. The
  command in "Purpose" uses `--include-ignored` instead, which runs both. This
  is exactly the failure mode #470 asks the lane to refuse, reached by a
  different route, and it is why EP-M5 asserts each expected model by name
  rather than only asserting a non-zero count.
- (2026-09-16, before implementation) The blocking problem is wider than the
  module documentation states. `rust_extension/tests/heavy/main.rs` attributes
  the abort to `std::thread::spawn` alone, but the handlers also use
  `crossbeam_channel` for records and acknowledgements and `parking_lot::Mutex`
  for interior state, neither of which Loom instruments. Replacing the spawn
  alone would move the failure from an abort to a deadlock inside Loom's
  executor. The seam in EP-M2 is sized accordingly.

## Decision log

- (2026-09-16) The plan is drafted and not executed. The agent's assignment
  permits an ExecPlan pull request only; no seam code is written until the user
  rules on the approach. This is recorded because the plan's first milestone
  deliberately leaves a scheduled lane red, which is a decision the user should
  make rather than discover.
- (2026-09-16) The timed receive has different semantics in the two
  configurations rather than the same semantics with different spellings.
  Alternatives considered: keeping a timeout under Loom, rejected because Loom
  has no clock and the timeout would fire on a schedule Loom chose, reporting a
  defect that does not exist; and removing the timeout from production,
  rejected because it is a real availability guard. The residual risk, that a
  liveness defect hangs the model instead of failing it, is bounded by the
  lane's job timeout in EP-M5.

## Progress

- [ ] EP-M1: make the absence visible
- [ ] EP-M2: the concurrency seam
- [ ] EP-M3: the stream handler onto the seam
- [ ] EP-M4: the file handler onto the seam
- [ ] EP-M5: the lane proves it ran
- [ ] EP-M6: say what is now true

Drafted 2026-09-16. Not started.

## Outcomes & retrospective

To be completed when the work is done.
