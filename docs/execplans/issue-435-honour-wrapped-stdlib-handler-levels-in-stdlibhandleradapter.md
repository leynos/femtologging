# Honour wrapped stdlib handler levels in `StdlibHandlerAdapter`

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: DRAFT

## Purpose / big picture

A femtologging handler that wraps a Python standard library (stdlib)
`logging.Handler` currently forwards every record the femtologging logger
accepts, ignoring the wrapped handler's own severity threshold. After this
change, an operator who writes
`StdlibHandlerAdapter(...). _handler.setLevel(logging.ERROR)` will see only
`ERROR` and `CRITICAL` records reach that handler, exactly as they would when
attaching the same handler to a stdlib `Logger`. The visible outcome is that
verbose low-severity records no longer leak into sensitive, expensive, or
high-severity-only destinations such as email, syslog, files, or network sinks.

## Constraints

Hard invariants that must hold throughout implementation. Violation requires
escalation, not a workaround.

- The wrapped handler's filters and its input-output (I/O) lock must be applied
  exactly once for every accepted record, and not at all for a rejected record.
  Concretely: `handle_record()` calls `logging.Handler.handle()` at most once,
  and never calls `emit()` directly.
- `logging.NOTSET` (numeric value `0`) semantics must be preserved. In stdlib,
  `Handler.setLevel(logging.NOTSET)` means "accept every record", because the
  `record.levelno >= handler.level` comparison is trivially true for a
  non-negative level number. The adapter must not special-case `NOTSET`; it
  must rely on the same comparison.
- The wrapped handler's `level` attribute must be read on every dispatch. Any
  `setLevel()` call made after the adapter is constructed must affect the very
  next record.
- The femtologging-to-stdlib level mapping, `_make_log_record()`, the
  warning-only `handle()` fallback, and the `flush()`/`close()` forwarders are
  unchanged behaviourally.
- `docs/users-guide.md` sections describing the `dictConfig` and `fileConfig`
  configuration schemas must not be modified except to add a single clarifying
  sentence that distinguishes schema-level handler-level restrictions from
  wrapped-handler behaviour. The schemas themselves are out of scope.
- Python documented style rules for this repository apply: en-GB-oxendict
  spelling in prose, module-level docstrings, docstrings on every public and
  private helper, and examples in docstrings that are exercised by
  `pytest --doctest-modules`.

## Tolerances (exception triggers)

Thresholds that trigger escalation when breached. These bound autonomous
action; they are not quality criteria.

- Scope: if implementation requires changes to more than 5 tracked files or
  more than 300 net lines, stop and escalate.
- Interface: if `StdlibHandlerAdapter.handle_record()` must change its
  signature, or any other public signature must change, stop and escalate.
- Dependencies: if a new external dependency is required, stop and escalate.
- Iterations: if a focused test still fails after 3 distinct fix attempts, stop
  and escalate with the failing output.
- Ambiguity: if the meaning of "levels are honoured" turns out to be
  self-contradictory (for example, if the repository's own tests assert that a
  wrapped handler receives records below its level), stop and present the
  contradiction with evidence.
- Time: if any single milestone exceeds 3 hours of wall-clock work, stop and
  report progress.

## Risks

Known uncertainties identified upfront, with mitigations.

- Risk: existing tests may assert the current (permissive) behaviour, so the
  change could break an unrelated suite.
  Severity: medium. Likelihood: low-medium. Mitigation: run the two adapter test
  modules plus the full `make test` suite; treat any newly failing test as
  evidence to investigate rather than to weaken.
- Risk: end-to-end adapter tests run through a worker thread and may be timing
  sensitive, producing flaky pass/fail.
  Severity: medium. Likelihood: medium. Mitigation: keep one test module
  entirely synchronous (direct `handle_record()` calls) and mark the
  thread-mediated module with an explicit timeout.
- Risk: rejecting a record before `handle()` means the handler's filter objects
  never run, which a reader might mistake for losing filter semantics.
  Severity: low. Likelihood: medium. Mitigation: match stdlib ordering exactly
  (`Logger.callHandlers()` gates on level before `Handler.handle()` runs
  filters) and state that ordering in the docstring, then prove the filter count
  contract with a dedicated counting-filter test.
- Risk: `docs/users-guide.md` and `docs/v0-1-0-migration-guide.md` drift out of
  sync with each other.
  Severity: low. Likelihood: medium. Mitigation: make the migration guide
  defer to the user guide section rather than restating it, and run
  `make markdownlint` (which enforces the `typos` spelling checker) plus
  `make nixie`.

## Progress

- [x] (2026-10-14 15:30Z) Renamed the working branch to
  `issue-435-honour-wrapped-stdlib-handler-levels-in-stdlibhandleradapter` and
  set the Lody session title.
- [x] (2026-10-14 15:35Z) Read `femtologging/adapter.py`,
  `tests/test_stdlib_adapter.py`, `tests/test_stdlib_adapter_records.py`, and
  the two affected documentation sections.
- [x] (2026-10-14 15:42Z) `make build` produced a testable environment.
  The build succeeded after roughly two minutes of compilation; the managed
  sccache server was unreachable so the build ran uncached, which is expected
  and not a failure.
- [ ] Reproduce the defect with a focused test run (Red). The defect itself is
  already reproduced directly; see `Artefacts and notes`.
- [ ] Implement the level gate in `handle_record()` (Green).
- [ ] Add the regression tests: level matrix, dynamic `setLevel()`, `NOTSET`,
  and the filter/`handle()` call-count contract.
- [ ] Update `docs/users-guide.md` and `docs/v0-1-0-migration-guide.md`.
- [ ] Run the full commit gates and a CodeRabbit review pass.
- [ ] Push the branch and open a draft pull request referencing the issue.

## Surprises & discoveries

- Observation: `docs/execplan/` and `docs/execplans/` both exist; the
  global instruction names `docs/execplans/`, and a repository grep finds
  `docs/execplans/issue-470-execute-loom-models.md` referenced from
  `docs/developers-guide.md`, so `docs/execplans/` is this repository's
  planning home. Impact: this plan lives in `docs/execplans/`.
- Observation: neither `StdlibHandlerAdapter` documentation section has an
  explicit Markdown anchor; the migration guide relies on the generated GitHub
  anchor `#using-stdlib-logginghandler-subclasses` derived from the heading
  text "Using standard library (stdlib) `logging.Handler` subclasses".
  Impact: when adding a new subsection under that heading, avoid placing a new
  heading whose text would change the existing generated anchor.

## Decision log

- Decision: Perform the red stage by adding the new tests first and confirming
  they fail against the unmodified adapter, rather than writing a throwaway
  reproducer script.
  Rationale: the tests are deliverables anyway, so the red stage doubles as
  the regression evidence required by the issue's acceptance criteria, and it
  avoids leaving scratch artefacts in the tree.
  Date/Author: 2026-10-14, implementing agent.
- Decision: Do not add a `@pytest.mark.xfail(strict=True)` marker to force the
  red stage.
  Rationale: the tests are added in the same commit series as the fix; a
  strict expected-failure marker would have to be added and then removed in the
  green step, and it would falsely imply an unresolved defect to a later
  reader. Running the new tests before the production edit demonstrates the
  failure directly, and the failing output is captured in this plan's
  `Artefacts and notes`.
  Date/Author: 2026-10-14, implementing agent.
- Decision: Read `handler.level` on every `handle_record()` call rather than
  caching it at construction.
  Rationale: a cached copy would break the documented stdlib pattern of
  calling `setLevel()` after the handler is attached, which is one of the
  issue's acceptance criteria.
  Date/Author: 2026-10-14, implementing agent.
- Decision: Place the new user-guide prose inside the existing
  `StdlibHandlerAdapter` section rather than adding a new `###` heading.
  Rationale: preserving heading text preserves the migration guide's existing
  cross-reference anchor.
  Date/Author: 2026-10-14, implementing agent.

## Outcomes & retrospective

To be completed at each milestone boundary and at completion. Must compare the
result against `Purpose / big picture` and reconcile every discovery with the
`Conformance basis`.

## Context and orientation

`femtologging` is a Rust-backed Python logging library. Its Python-facing
handlers are dispatched through a `handle_record(record)` method where
`record` is a plain dictionary, and handler work runs on dedicated worker
threads. The standard library's `logging.Handler` subclasses instead expect an
`emit(LogRecord)` method. `femtologging/adapter.py` provides
`StdlibHandlerAdapter`, a Python class that bridges the two interfaces.

The relevant files are:

- `femtologging/adapter.py` — the adapter itself. `StdlibHandlerAdapter`
  wraps a `logging.Handler`, `handle_record()` translates the femtologging
  dictionary into a `logging.LogRecord` via the module-level helper
  `_make_log_record()`, and `_stdlib_levelno()` maps the femtologging level to
  a stdlib numeric level (TRACE 5, DEBUG 10, INFO 20, WARN 30, ERROR 40,
  CRITICAL 50). The wrapped handler is stored as `self._handler`.
- `tests/test_stdlib_adapter.py` — construction, dispatch, and level-mapping
  tests. It builds an `AdapterProbe` fixture (a `StringIO` stream, a
  `logging.StreamHandler` over it, and the adapter) and drives records through a
  real `FemtoLogger` using the `_LogRequest` and `_HandlerSetup` dataclasses.
- `tests/test_stdlib_adapter_records.py` — `LogRecord` attribute, delegation,
  and export tests. It defines `CapturingHandler`, a `logging.Handler` subclass
  that appends emitted records to `self.records`, plus the synchronous helpers
  `_emit_femto_record()` and `_emit_via_logger()`.
- `docs/users-guide.md` — the section "Using standard library (stdlib)
  `logging.Handler` subclasses" describes the adapter's behaviour and
  limitations, and later sections describe the restricted `dictConfig` and
  `fileConfig` schemas (which reject handler `level` in configuration for
  unrelated reasons).
- `docs/v0-1-0-migration-guide.md` — the section "New: standard library
  (stdlib) logging handler adapter" introduces the adapter and links to the
  users guide.

Two different thresholds are in play, and the fix is about keeping them
distinct:

1. The femtologging logger threshold. `FemtoLogger.set_level(...)` (and the
   Rust-side logger configuration) decides whether the *logger* accepts a
   record at all. This happens before any handler sees anything and is outside
   the adapter's control.
2. The wrapped stdlib handler threshold. `logging.Handler.setLevel(...)` sets
   `handler.level`. In the standard library this is enforced by
   `Logger.callHandlers()`, which checks `record.levelno >= handler.level`
   before calling `handler.handle()`. `Handler.handle()` itself only applies
   the handler's `filters` and its I/O lock; it never checks `level`. Because
   the adapter calls `handler.handle()` directly, threshold (2) is currently
   never enforced.

The term "wrapped handler" below always means the `logging.Handler` instance
held by `StdlibHandlerAdapter` in `self._handler`.

## Conformance basis

This repository does not maintain a separate Terms of Reference or
technical-design document for this bug fix. The upstream artefacts are:

- GitHub issue `leynos/femtologging#435`, "Honour wrapped stdlib handler levels
  in `StdlibHandlerAdapter`", which carries the acceptance criteria used here.
- CPython's `logging` module as the governing behavioural standard. The
  relevant reference behaviour is `Logger.callHandlers()` in
  `Lib/logging/__init__.py`, whose handler loop begins with
  `if record.levelno >= hdlr.level:`. The adapter reproduces the part of that
  contract that applies to a single handler.
- `docs/adr-003-python-stdlib-filter-parity.md`, which governs how Python
  stdlib parity work is sequenced in this project. Its "Non-goals" section
  explicitly defers handler-level stdlib filter parity; that restriction is
  about *filters*, not about *severity levels*, so this change does not
  contradict it. ADR-003's technical requirement "Preserve current handler-level
  filter constraints unless explicitly added in a separate decision" is
  satisfied: this plan adds no handler-level filter support.

Trace links:

```plaintext
Issue#435-AC1 (reject below handler level)     -> EP-M1 -> TestHandlerLevelThreshold::test_error_handler_rejects_below_threshold
Issue#435-AC2 (preserve NOTSET semantics)      -> EP-M1 -> TestHandlerLevelThreshold::test_notset_handler_accepts_every_level
Issue#435-AC3 (filters and lock exactly once)  -> EP-M1 -> TestHandleRecordFilterAndLockContract::test_handle_and_filter_counts_by_level
Issue#435-AC4 (ERROR handler level matrix)     -> EP-M1 -> TestHandlerLevelThreshold::test_error_handler_rejects_below_threshold
Issue#435-AC5 (dynamic setLevel)               -> EP-M1 -> TestDynamicHandlerLevel::test_set_level_after_construction_applies
Issue#435-AC6 (document both thresholds)       -> EP-M2 -> docs/users-guide.md, docs/v0-1-0-migration-guide.md
```

## Verification plan

The behaviour under change is a single guard in one method. It introduces one
invariant and one ordering lemma, and it depends on one non-trivial axiom about
the standard library. No non-trivial proof obligation arises beyond these, and
an attempt to use a formal prover here would be disproportionate: the guard is
a single comparison whose correctness is established exhaustively by a finite
partition, because the input domain (six femtologging levels crossed with a
small set of handler levels) is small enough to enumerate completely.

Axiom A1: in CPython's `logging`, `Handler.handle()` applies
`self.filters` and acquires `self.lock`, and does *not* consult `self.level`.
Repository-owned code depends on this interaction, so it is verified at the
contract boundary rather than assumed from documentation alone — by asserting
observable call counts and lock acquisition through the real stdlib types. The
environment assumption is the repository's pinned interpreter (Python 3.12+)
with the stdlib `logging` module unmodified; no monkeypatching of `logging`
occurs in these tests.

Invariant I1 (level gate): for every constructed `LogRecord` with numeric level
`L` and every wrapped-handler level `H`, the adapter emits the record if and
only if `L >= H`; equivalently it never invokes `emit()` when `L < H`, and it
never suppresses a record when `L >= H`. This is the complete specification of
the change.

Lemma L1 (ordering): the level test is evaluated before any other observable
side effect of dispatch. Consequences: (a) the number of `Handler.handle()`
invocations equals the number of accepted records; (b) `Handler.handle()`
invocations for accepted records each apply the handler's filter chain exactly
once; (c) the handler's I/O lock is acquired exactly once per accepted record
and never for a rejected record.

Invariant I2 (live configuration): the gate reads `handler.level` at dispatch
time, so for a fixed adapter instance a `setLevel()` call between two
`handle_record()` calls changes the verdict of the second call without
reconstructing the adapter.

- Obligation: I1 for a handler fixed at `logging.ERROR`.
  Method: parameterized test over the finite partition of all six femtologging
  levels.
  Rationale: the partition is total over the adapter's level vocabulary and
  each row has distinct semantic meaning (five rejects, one accept at the
  boundary, plus `CRITICAL` above it), so enumeration is exhaustive rather than
  a sample.
  Domain: handler level `logging.ERROR`; logger level `"TRACE"`; record levels
  `TRACE`, `DEBUG`, `INFO`, `WARNING`, `ERROR`, `CRITICAL`.
  Artefact: `tests/test_stdlib_adapter.py`,
  `TestHandlerLevelThreshold::test_error_handler_rejects_below_threshold`.
  Evidence: before the change, every row emits and the five reject rows fail;
  after the change, all six rows pass. Discharge condition: the five
  below-`ERROR` rows produce empty captured output and the two at-or-above rows
  produce exactly the expected formatted line.
  Non-vacuity: the logger threshold is `"TRACE"`, the most permissive setting,
  so no row is pre-filtered by the logger and every rejection is attributable
  to the new handler-level gate. The `WARNING` row is the nearest miss
  immediately below the boundary and the `ERROR` row is the boundary itself, so
  both sides of the comparison are exercised. The negative control is the `red`
  run recorded in `Artefacts and notes`, where the identical test body succeeds
  only for rows whose expected outcome the broken code happens to match.

- Obligation: I1 at the `NOTSET` boundary, and the equivalence of "handler
  level `NOTSET`" with "accept every mapped level".
  Method: named test, plus a second named test asserting the pre-existing
  default-handler case in the sibling module.
  Rationale: `NOTSET` is the one value where a plausible "simplification" (for
  example `if self._handler.level:` truthiness) would silently regress, because
  `NOTSET` is numeric zero and therefore falsy. A named regression test is the
  proportionate guard.
  Domain: handler level `logging.NOTSET`; records at `TRACE` (stdlib level 5,
  the lowest mapped level) and `INFO`; logger thresholds `"TRACE"` and default.
  Artefact: `tests/test_stdlib_adapter.py`,
  `TestHandlerLevelThreshold::test_notset_handler_accepts_every_level`, and
  `tests/test_stdlib_adapter_records.py`,
  `TestDynamicHandlerLevel::test_set_level_after_construction_applies`.
  Evidence: all accepted records appear with stdlib level numbers 5 and 20
  respectively.
  Non-vacuity: `TRACE` maps to the lowest nonzero numeric level in the mapping,
  so if `NOTSET` were treated as a hard threshold rather than as "no
  threshold", this row is the first to fail. The truthiness hazard is a real
  reachable defect, not a hypothetical one.

- Obligation: L1, covering both the "exactly once" and the "not at all" halves.
  Method: named tests using a counting handler subclass and a counting
  `logging.Filter` subclass.
  Rationale: this is a side-effect-ordering claim, not an input/output claim,
  so counting observed invocations through the real stdlib objects is a direct
  and stronger observation than inspecting output text.
  Domain: handler level `logging.ERROR`; one accepted record (`ERROR`), one
  rejected record (`DEBUG`), and one record that passes the level gate but is
  rejected by a filter.
  Artefact: `tests/test_stdlib_adapter.py` (or
  `tests/test_stdlib_adapter_records.py`),
  `TestHandleRecordFilterAndLockContract`.
  Evidence: after the folded-in rejection, a `DEBUG` record leaves the
  `handle()` count and the filter-call count unchanged and emits nothing; an
  `ERROR` record increments both by exactly one and emits exactly one record;
  the filter-rejected `ERROR` record increments `handle()` once, increments the
  filter count once, and emits nothing.
  Non-vacuity: the test asserts counts rather than "not zero", so an
  implementation that calls `handle()` twice or that skips filters entirely
  fails. The filter-rejected row proves the filter chain is genuinely reached
  through `Handler.handle()`, so "the gate ran but the filter never did" cannot
  pass unnoticed.
  Residual gap: the handler's I/O lock is exercised implicitly by the real
  `Handler.handle()` implementation rather than asserted directly, because the
  stdlib does not expose lock acquisition as a public observable. A test that
  monkeypatched the lock would verify the monkeypatch, not the stdlib.

- Obligation: I2 for a live `setLevel()` sequence.
  Method: named test over an ordered sequence of three dispatches with two
  intervening `setLevel()` calls.
  Rationale: this is an ordering property over a short operation sequence
  rather than a property over generated inputs; enumerating the three states
  explicitly is clearer than generating them, and the sequence is short enough
  to be read as a specification.
  Domain: adapter constructed with a `NOTSET` handler; `INFO` dispatch;
  `setLevel(logging.ERROR)` then `INFO` and `ERROR` dispatches;
  `setLevel(logging.NOTSET)` then a `TRACE` dispatch.
  Artefact: `tests/test_stdlib_adapter_records.py`,
  `TestDynamicHandlerLevel::test_set_level_after_construction_applies`.
  Evidence: discharges to exactly the records `INFO`, then `ERROR`, then
  `TRACE`, with the final `TRACE` record reporting `levelno == 5`.
  Non-vacuity: the sequence tests all three transitions that a cached level
  could get wrong — loosening, tightening, and returning to `NOTSET`. A
  construction-time cache fails the second dispatch; a construction-time cache
  that happens to work because the handler was already `NOTSET` still fails the
  third.

## Plan of work

The change is small enough to be a single behavioural unit, so there are two
implementation milestones with a documentation milestone alongside.

Stage A (understanding) is complete: the defect, the two test modules, and the
two documentation sections have been read, and the CPython reference behaviour
(`Logger.callHandlers()`) has been identified as the specification to match.

Stage B (red) adds the new tests to the two existing test modules and runs them
against the unmodified adapter to confirm they fail for the intended reason.

Stage C (green) makes the smallest production change that satisfies the tests:
one guard in `StdlibHandlerAdapter.handle_record()`.

Stage D (documentation and wider validation) updates the two documentation
files and runs the full commit gates.

### Milestone EP-M1: the level gate is enforced and proven

Outcome: `StdlibHandlerAdapter` rejects records below the wrapped handler's
live level, accepts everything else, and applies filters and the lock exactly
once to accepted records. The repository is coherent on its own: the adapter
and its tests agree.

Changes:

- `femtologging/adapter.py`: in `handle_record()`, after
  `log_record = _make_log_record(record)`, return early when
  `log_record.levelno < self._handler.level`. Update the class docstring and
  the `handle_record()` docstring to state that the adapter honours both the
  femtologging logger threshold and the wrapped handler's level, that the
  comparison happens before `handle()` runs filters and takes the I/O lock, and
  that `setLevel()` changes apply to the next record.
- `tests/test_stdlib_adapter.py`: add `TestHandlerLevelThreshold` with the
  six-row level matrix against an `ERROR` handler under a `"TRACE"` logger, and
  the `NOTSET` acceptance test.
- `tests/test_stdlib_adapter_records.py`: add `TestDynamicHandlerLevel` with
  the synchronous `setLevel()` sequence. This module is chosen for the dynamic
  test because `_emit_femto_record()` calls `handle_record()` directly, which
  makes the sequence deterministic with no worker-thread timing involvement.
- Add a counting handler and a counting filter helper where the call-count
  contract test lives.

Acceptance evidence: the two focused test modules pass, and the red run for
the same tests before the production change is recorded in `Artefacts and
notes`.

Conformance check:

- Requirements satisfied: Issue#435 acceptance criteria 1 through 5.
- Design still followed: the adapter keeps its single `handle_record` dispatch
  path and still delegates to `Handler.handle()`.
- Upstream assumptions still valid: CPython `Handler.handle()` semantics per
  Axiom A1.
- No unapproved public interface or dependency change: `handle_record()`
  signature and the package's public exports are unchanged.
- No unapproved trust boundary or persisted-format change: none introduced.
- Trace links current: the EP-M1 links in `Conformance basis`.

Recovery: the milestone is one guard plus tests; `git revert` of the milestone
commit restores the previous behaviour exactly.

Remaining gaps: documentation (EP-M2).

Compatibility decision: none required. The wrapped handler's `level` is
public, documented stdlib surface, and the femtologging package is pre-1.0
(`version = "0.1.0"` in `pyproject.toml`). No compatibility layer is
introduced.

### Milestone EP-M2: both thresholds are documented

Outcome: a reader of the users guide understands that two independent
thresholds apply, in which order, and how the wrapped-handler threshold relates
to the unrelated configuration-schema restriction on handler levels; the
migration guide points at the same explanation instead of restating it.

Changes:

- `docs/users-guide.md`: extend the `StdlibHandlerAdapter` section with prose
  and a short example showing a `"TRACE"` femtologging logger with a
  `logging.ERROR` handler, plus one sentence distinguishing the
  configuration-schema handler-level restriction from wrapped-handler
  behaviour.
- `docs/v0-1-0-migration-guide.md`: add one or two sentences stating that the
  adapter honours both thresholds and link to the users guide section for
  details.

Acceptance evidence: `make markdownlint` passes (including the `typos`
en-GB-oxendict check) and `make nixie` passes.

Conformance check:

- Requirements satisfied: Issue#435 acceptance criterion 6.
- Design still followed: documentation describes the shipped behaviour.
- Upstream assumptions still valid: the existing user-guide heading text is
  unchanged, so the migration guide's anchor still resolves.
- No unapproved public interface or dependency change: none.
- No unapproved trust boundary or persisted-format change: none.
- Trace links current: the EP-M2 link in `Conformance basis`.

Recovery: documentation-only; `git revert` of the milestone commit.

Remaining gaps: none.

Compatibility decision: none required.

## Concrete steps

All commands run from the worktree root
`/home/leynos/.lody/repos/github---leynos---femtologging/worktrees/e4b3b11d-b949-4f71-a492-dec71a2a10c7`.
Prefix each gate with output capture as the repository conventions require, for
example:

```plaintext
make <target> 2>&1 | tee /tmp/<action>-femtologging-$(git branch --show-current).out
```

Step 1 — build the development environment (creates `.venv` and installs the
compiled extension). Expect a successful `maturin develop`:

```plaintext
make build
```

Step 2 — add the new tests, then run them *before* touching
`femtologging/adapter.py`. Expect failures on the below-threshold rows:

```plaintext
uv run pytest tests/test_stdlib_adapter.py tests/test_stdlib_adapter_records.py -v
```

Step 3 — apply the guard in `femtologging/adapter.py` and re-run the same two
modules. Expect all tests to pass:

```plaintext
uv run pytest tests/test_stdlib_adapter.py tests/test_stdlib_adapter_records.py -v
```

Step 4 — commit the production change and its tests as one atomic commit.

Step 5 — update the documentation, then validate it:

```plaintext
make markdownlint
make nixie
```

Step 6 — commit the documentation as a separate atomic commit.

Step 7 — run the full commit gates through the `scrutineer` subagent (format
check, lint, typecheck, tests, Markdown lint, Mermaid validation), then request
a `coderabbit review --agent` pass and clear every finding before proceeding.

Step 8 — push over SSH and open a draft pull request.

## Validation and acceptance

Behavioural acceptance is expressed as the captured output of a runnable
program. With the wrapped handler set to `logging.ERROR`, a femtologging logger
at `"TRACE"`, and a `%(levelname)s %(message)s` formatter, logging one message
at each of the six levels must produce exactly two lines:

```plaintext
ERROR e
CRITICAL c
```

Before the change the same program produces six lines. The corresponding
automated check is the parameterized test named in the verification plan.

Quality criteria:

- Tests: `make test` passes, including the new tests. The new tests fail before
  the production change and pass after.
- Verification: Invariants I1 and I2 and Lemma L1 are discharged as described
  in `Verification plan`; Axiom A1 is exercised at its real boundary.
- Lint/typecheck: `make check-fmt`, `make lint` and `make typecheck` all pass
  with no warnings.
- Performance: not applicable; the change adds one integer comparison and one
  attribute read per dispatched record on a path that already builds a
  `LogRecord`.
- Security: none; the change strictly reduces the set of records leaving the
  process.

Quality method: `scrutineer` runs the gates sequentially and reports the
captured per-gate logs under `/tmp`, followed by a `coderabbit review --agent`
pass.

## Idempotence and recovery

Every step is safe to repeat. `make build` recreates `.venv` and reinstalls the
extension. The test commands are read-only with respect to tracked files. Both
milestone commits are single logical units that can be reverted independently
with `git revert`.

If a gate fails, read the captured log under `/tmp` rather than re-running the
gate, apply the fix, and re-run. If the build queue is congested, wait for the
shared compile slots; a queue wait is not a failure.

## Artefacts and notes

### Manual reproduction of the defect, 2026-10-14

Run against the unmodified adapter with the development environment built from
`make build`. A `logging.StreamHandler` set to `logging.ERROR` receives an
`INFO` record from a `"TRACE"` femtologging logger:

```plaintext
$ .venv/bin/python -c "
from femtologging import FemtoLogger, StdlibHandlerAdapter
import logging, io
h = logging.StreamHandler(io.StringIO())
h.setLevel(logging.ERROR)
a = StdlibHandlerAdapter(h)
l = FemtoLogger('probe'); l.set_level('TRACE'); l.add_handler(a)
l.log('INFO', 'info-should-be-suppressed'); del l
print('captured:', repr(h.stream.getvalue()))
"
captured: 'info-should-be-suppressed\n'
```

Expected after the fix: `captured: ''`.

The red-stage transcript and the green-stage transcript of the focused test
modules belong here too, along with the gate summaries and any CodeRabbit
findings and their resolutions.

## Interfaces and dependencies

No new dependencies and no signature changes. The one production-side interface
that matters is the existing method, whose contract is extended rather than
altered:

```python
class StdlibHandlerAdapter:
    def handle_record(self, record: FemtoRecord) -> None:
        """Translate a femtologging record and dispatch it if the handler accepts it.

        Records whose mapped stdlib level is below the wrapped handler's
        ``level`` are discarded before ``handle()`` runs, matching
        ``logging.Logger.callHandlers()``.  Accepted records are passed to
        ``self._handler.handle()`` exactly once, which applies the handler's
        filters and I/O lock.
        """
```

`FemtoRecord` is the existing `TypedDict` in `femtologging/adapter.py`; the
method keeps returning `None`, and rejection is signalled by the absence of a
side effect rather than by an exception or a return value, exactly as
`logging.Logger` does.

## Revision note

(Required when editing an ExecPlan.) Initial draft created 2026-10-14.
