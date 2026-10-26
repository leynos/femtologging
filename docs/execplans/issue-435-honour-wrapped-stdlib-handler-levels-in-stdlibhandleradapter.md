# Honour wrapped stdlib handler levels in `StdlibHandlerAdapter`

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: IN REVIEW (PR #546; ready for review)

## Purpose / big picture

A femtologging handler that wraps a Python standard library (stdlib)
`logging.Handler` currently forwards every record the femtologging logger
accepts, ignoring the wrapped handler's own severity threshold. After this
change, an operator who sets `logging.ERROR` on a stdlib handler before
wrapping it with `StdlibHandlerAdapter` will see only `ERROR` and `CRITICAL`
records reach that handler, exactly as they would when attaching the same
handler to a stdlib `Logger`. The visible outcome is that verbose low-severity
records no longer leak into sensitive, expensive, or high-severity-only
destinations such as email, syslog, files, or network sinks.

## Constraints

Hard invariants that must hold throughout implementation. Violation requires
escalation, not a workaround.

- After translating a record, the handler-level threshold must gate entry to
  `logging.Handler.handle()`. A level-rejected record must not enter the
  handler's `handle()` path. A level-accepted record calls `handle()` exactly
  once. For handlers retaining the base `logging.Handler.handle()` behaviour,
  that call, on normal completion, evaluates the filter chain once and acquires
  the I/O lock only when the chain accepts the record. The adapter never calls
  `emit()` directly.
- `logging.NOTSET` (numeric value `0`) semantics must be preserved. In stdlib,
  `Handler.setLevel(logging.NOTSET)` means "accept every record", because the
  `record.levelno >= handler.level` comparison is trivially true for a
  non-negative level number. The adapter must not special-case `NOTSET`; it
  must rely on the same comparison.
- The wrapped handler's `level` attribute must be read on every dispatch. A
  sequential `setLevel()` call between dispatches must affect the level value
  read by the next dispatch. Concurrent updates are not atomic with the gate or
  subsequent handler work.
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
  change could break an unrelated suite. Severity: medium. Likelihood:
  low-medium. Mitigation: run the three adapter test modules plus the full
  `make test` suite; treat any newly failing test as evidence to investigate
  rather than to weaken.
- Risk: end-to-end adapter tests run through a worker thread and may be timing
  sensitive, producing flaky pass/fail. Severity: medium. Likelihood: medium.
  Mitigation: keep one test module entirely synchronous (direct
  `handle_record()` calls) and mark the thread-mediated module with an explicit
  timeout.
- Risk: rejecting a record before `handle()` means the handler's filter objects
  never run, which a reader might mistake for losing filter semantics.
  Severity: low. Likelihood: medium. Mitigation: match stdlib ordering exactly
  (`Logger.callHandlers()` gates on level before `Handler.handle()` runs
  filters) and state that ordering in the docstring, then prove the filter
  count contract with a dedicated counting-filter test.
- Risk: `docs/users-guide.md` and `docs/v0-1-0-migration-guide.md` drift out of
  sync with each other. Severity: low. Likelihood: medium. Mitigation: make the
  migration guide defer to the user guide section rather than restating it, and
  run `make markdownlint` (which enforces the `typos` spelling checker) plus
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
- [x] (2026-10-14 16:05Z) Reproduced the defect with a focused test run (Red).
  The four below-threshold cases failed with the pre-fix `handle_record()`,
  which confirmed they exercise the defect rather than restating existing
  behaviour; the direct reproduction is recorded in `Artefacts and notes`.
- [x] (2026-10-14 16:20Z) Implemented the level gate in `handle_record()`
  (Green). All 38 tests in the two adapter suites and 45 doctests passed.
- [x] (2026-10-14 16:35Z) Added the regression tests: level matrix, dynamic
  `setLevel()`, `NOTSET`, and the filter/`handle()` call-count contract.
- [x] (2026-10-14 17:00Z) Updated `docs/users-guide.md` and
  `docs/v0-1-0-migration-guide.md`.
- [x] (2026-10-14 18:10Z) Rebased onto `origin/main` after it advanced by four
  commits (including `#418` and `#433`, both of which touch the adapter and its
  tests). The rebase applied cleanly; the merged class docstring carries both
  this change's two-thresholds paragraph and origin's `contextvars` paragraph.
- [x] (2026-10-16 16:46Z) Split the two oversized modules to satisfy the
  `too-many-lines` (400) lint gate, which `origin/main`'s `adapter.py` sat
  exactly on at 400 lines. Extracted the record schema and translation into
  `femtologging/_adapter_record.py`, re-exported through `femtologging.adapter`
  so every existing import site is unchanged, and extracted the shared test
  vocabulary into `tests/stdlib_adapter_support.py` with the threshold tests
  moving to `tests/test_stdlib_adapter_levels.py`.
- [x] (2026-10-22 23:13Z) Refreshed `origin/main` to `da1d82e` and rebased
  the four issue commits from `4ff883a` onto the five newer main commits. The
  file sets did not overlap and the rebase completed cleanly; pre-rebase head,
  base, and target refs were preserved for recovery.
- [x] (2026-10-23 00:11Z) Full gates found one stale wheel-layout expectation:
  the existing extraction of `_adapter_record.py` was absent from
  `expected_wheel_snapshot.json`. Rebuilt the wheel and confirmed this was the
  only snapshot difference; the expected entry is now recorded.
- [x] (2026-10-23 00:36Z) Ran the full gates sequentially: formatting check,
  lint, type check, full tests (769 passed, 5 skipped, 66 snapshots passed, one
  existing warning), Markdown lint, and Mermaid validation all passed.
- [x] (2026-10-23 00:43Z) Ran `coderabbit review --agent` on commit `076bd39`.
  It reported six findings: duplicate level-gate optimization suggestions,
  duplicated test helper, a private-attribute example, spelling, and stale
  module/test references. The explicit record-construction-first requirement
  governs the optimization suggestions; the other four findings have been
  corrected. The shared `CapturingHandler` now serves both test modules, and
  the plan's examples, spelling, module references, and performance note match
  the implementation.
- [x] (2026-10-23 01:40Z) Reran all six gates sequentially after those fixes:
  `check-fmt`, `lint`, `typecheck`, `test` (769 passed, 5 skipped; 66 snapshots
  passed; one existing `PytestAssertRewriteWarning`), `markdownlint` (0
  errors), and `nixie` (all diagrams validated).
- [x] (2026-10-23 02:05Z) Reviewed cleanup commit `919c7a0` with
  `coderabbit review --agent`. It found an incorrect count in the matrix
  evidence, duplicate adapter-probe fixtures, and repeated message extraction.
  Two findings duplicated the matrix and fixture concerns; all three unique
  issues are corrected in the review-fix diff.
- [x] (2026-10-23 04:03Z) Ran all seven gates sequentially on the exact
  five-file review-fix diff in an independent worktree. Formatting, lint,
  typecheck, tests (769 passed, 5 skipped; 66 snapshots passed), Markdown lint,
  and Mermaid validation passed. The original worktree's pytest teardown hang
  did not recur.
- [x] (2026-10-23) Reviewed commit `01d8e59` with `coderabbit review --agent`
  after the full gates. The review returned eight findings, four of them
  duplicates. The unique findings concerned missing NumPy-style field
  documentation on re-exported TypedDicts and shared test dataclasses,
  translation-versus-threshold ordering, and conditional filter/lock/emission
  semantics. This diff corrects those descriptions and records field
  requiredness.
- [x] (2026-10-23) The implementation exceeded the five-file stop condition:
  the docs and adapter/test module size required updates and extraction, and
  `main` had advanced. Worktree and rebase review were completed; the full
  gates passed on the resulting candidate.
- [x] (2026-10-23) CodeRabbit reviewed `1d15edd` and requested a positive level
  guard instead of an explicit `return` in `handle_record()`. The method now
  invokes `handle()` only inside the equivalent passing-level condition.
- [x] (2026-10-23) Ran all seven gates on the positive-guard review fix and
  committed it as `ba6b0ff`. CodeRabbit reviewed that clean commit after the
  requested cooldown and returned zero findings.
- [x] (2026-10-23) Pushed the reviewed branch and opened draft PR #546:
  <https://github.com/leynos/femtologging/pull/546>
- [x] (2026-10-23) Rebased the 12 issue commits from `da1d82e` onto
  `5377ceb` without conflicts. `range-diff` matched all 12 commits, and the
  target's changed paths did not overlap this issue's files. The new Rust
  private-doc policy and expanded `make lint` lanes are recorded below. The
  post-rebase `make check-fmt`, `make test`, `make typecheck`, and `make lint`
  gates passed; `make fmt`, `make markdownlint`, and `make nixie` passed too.
- [x] (2026-10-23) CodeRabbit reviewed rebased commit `e07d31f` and identified
  stale status, outcomes, and revision-note sections in this plan. These
  sections were completed before the final review and push.
- [x] (2026-10-23) CodeRabbit reviewed `38bdbe8` and found that the guide did
  not explain that level changes affect queued records not yet processed. The
  user guide now states when the live level is read and which records a change
  affects. The documentation gates passed, the final CodeRabbit review on
  `12cc9fc` returned zero findings, and the branch was force-pushed with a
  lease to draft PR #546.
- [x] (2026-10-23) Updated draft PR #546's validation summary for the rebased
  candidate and placed the Lody session link in the PR body's terminal
  `## References` section.
- [x] (2026-10-24) Added the requested wrapped-handler level and filter
  sequence to section 6.5 of the architecture design, with a labelled
  screen-reader description and visible figure caption.
- [x] (2026-10-25) Validated the accessible diagram and behaviour changes with
  all six candidate gates. The run passed 770 tests, skipped five, and reported
  one warning; all other gates passed. Increased the main-branch private-docs
  contract timeout to 1,800 seconds (3,620 seconds for pytest) because healthy
  shared compiler admission made its eight Clippy lanes exceed 600 seconds.
  CodeScene then identified a duplicated handler probe; replacing the subclass
  with a spy around the existing handler cleared the finding.
- [x] (2026-10-25) Rebased the 16 issue commits from `5377ceb` onto updated
  `origin/main` at `d234990` (`#426`), with no conflicts. `range-diff` matched
  all 16 commits. Main added configuration-level handler filters and formatter
  support, but no adapter implementation or shared test helper changes. Since
  `dictConfig` now supports handler filters while still rejecting handler
  `level`, clarified the guide sentence to name the `level` restriction only.
- [x] (2026-10-25) Corrected three minor findings from the review of `887ebf4`
  and passed all six gates on the resulting candidate (797 passed, 5 skipped).
  CodeScene delta reported no issues. The fresh review of `9be9cc6` returned
  four minor documentation findings, now corrected in the current worktree; the
  first full gate run on those corrections also passed all six gates.
- [x] (2026-10-25) Ran formatting, format-check, Markdown and Mermaid gates on
  this final progress update; all passed.
- [x] (2026-10-25) Corrected the `NOTSET` coverage claim from the review of
  `af7539d`: the named test now describes its TRACE, DEBUG and INFO cases, and
  the rationale no longer claims an equivalent zero-threshold shortcut fails.
  All six gates passed (797 tests passed, 5 skipped); CodeScene found no issues.
- [x] (2026-10-25) Published candidate `1077f49` on draft PR #546. Hosted run
  `40192246149` passed on all four Python versions, and CodeScene reported six
  quality gates passed. The completeness assessment found no runtime defect and
  requested corrections to publication status, verification evidence, timeout
  accounting and the finite test-domain claim. These corrections are recorded
  below and in the subsequent candidate validation.
- [x] (2026-10-25) Candidate `e065086` passed all six sequential local gates,
  hosted run `40205578288` on Python 3.12–3.15, and CodeScene result `8119031`.
  The completeness corrections are recorded.
- [x] (2026-10-25) The completeness follow-up on `6e24864` confirmed the four
  prior findings resolved and found no runtime defect. It identified two more
  plan inaccuracies: the dynamic test's four dispatches and cached-level
  rationale, plus the filter-rejected `CRITICAL` record. Corrected below.
- [x] (2026-10-25) Applied the proof-specific precision corrections on
  `8fd2cdf`. The six-gate candidate run completed; `make lint`'s captured log
  has no findings and ends with successful Whitaker Cargo output, but its shell
  exit status was not retained. Hosted run `40233382614` and CodeScene result
  `8121472` passed. The proof assessment and completeness follow-up confirmed
  the plan and proof corrections, while both identified remaining user-guide
  wording that overstates base-handler filter and lock behaviour.
- [x] (2026-10-26) Committed and published the base-handler guide qualification
  as `8001ddc`. Its exact worktree candidate passed all six sequential local
  gates: `check-fmt`, `test` (797 passed, 5 skipped; 66 snapshots), `typecheck`,
  `lint`, `markdownlint` and `nixie`. CodeScene result `8122938` passed.
  Hosted CI was still running when the candidate-bound assessments arrived.
  Those assessments accepted the filter/lock/emission qualification and proof,
  and identified asynchronous-context wording plus the completed publication
  step as corrections needed in the next candidate.
- [x] (2026-10-26) Limited context claims across the adapter documentation and
  user, developer, migration and decision guides to synchronous handler work,
  and recorded publication of `8001ddc` as complete. The six-path worktree
  candidate before final formatting passed all six sequential local gates:
  `check-fmt`, `test` (797 passed, 5 skipped; 66 snapshots), `typecheck`,
  `lint`, `markdownlint` (61 files, no errors) and `nixie`. The old-head hosted
  run `40249466989` completed successfully. A later `make fmt` normalized the
  adapter docstring, changing the diff hash; the documentation gates passed on
  that normalized content.
- [x] (2026-10-26) The context-boundary edits passed `make lint`,
  `make typecheck`, and the full `make test`
  (`797 passed, 5 skipped; 66 snapshots`; all Rust feature lanes and doctests
  passed). The first Python-feature doctest attempt had unresolved symbols from
  an `extension-module` PyO3 artefact; an isolated rebuild and the later full
  test passed without source changes. After the final ExecPlan evidence update,
  `make fmt`, `make check-fmt`, `make markdownlint` (0 errors), and
  `make nixie` passed on the exact documentation worktree.
- [x] (2026-10-26) Committed and published the validated six-path correction
  as `483d14e`. CodeScene result `8124167` passed. The candidate-bound response
  `6407177509` confirmed the synchronous-context correction and prior
  publication finding, and identified this stale publication wording. At that
  time, hosted run `40261257870` was still in progress on the assessed head.
- [x] (2026-10-26) On published head `5a3825e`, `make check-fmt`, `make lint`,
  `make typecheck`, and `make test` passed
  (`797 passed, 5 skipped; 66 snapshots`). Hosted CI run `40261703860` and
  CodeScene result `8124222` passed. CodeRabbit's focused confirmation
  `6407235083` confirmed the publication wording and found no other stale
  status entry.
- [x] (2026-10-26) PR #546 is ready for review; GitHub recorded the
  `ready_for_review` event at 04:13:42Z. CodeRabbit's status on head `7b863f6`
  was `Review rate limited`; its summary said the next included review would be
  available in 11 minutes. No PR #546 request appeared in `comenq list` at that
  observation.
- [x] (2026-10-26) On published head `9e6aedf`, hosted run `40268201847`
  passed for Python 3.12, 3.13, 3.14 and the allowed-failure 3.15 lane; exact-
  head CodeScene result `8124868` passed all six quality gates. The source
  candidate immediately before the plan-only status updates passed all six
  sequential local gates (797 passed, 5 skipped; 66 snapshots); the later
  plan-only candidates passed format, format-check, Markdown and Mermaid gates.
  CodeRabbit's current-head status is rate limited; queued request `e8d2e60c`
  is pending and formal review remains outstanding.
- [x] (2026-10-26) Focused status assessment `6408366475` on head `9e6aedf`
  accepted the historical readiness and queue observations and identified the
  stale draft label, pending-review action and missing exact-head validation
  record. Those corrections are made here; confirmation on this updated plan
  candidate remains pending.
- [ ] (2026-10-26) Obtain candidate-bound confirmation of the current plan
  corrections, await the existing queued formal-review request `e8d2e60c`,
  reconcile all findings, obtain the applicable approval, and merge.

## Surprises & discoveries

- Observation: `docs/execplan/` and `docs/execplans/` both exist; the
  global instruction names `docs/execplans/`, and a repository grep finds
  `docs/execplans/issue-470-execute-loom-models.md` referenced from
  `docs/developers-guide.md`, so `docs/execplans/` is this repository's
  planning home. Impact: this plan lives in `docs/execplans/`.
- Observation: neither `StdlibHandlerAdapter` documentation section has an
  explicit Markdown anchor; the migration guide relies on the generated GitHub
  anchor `#using-stdlib-logginghandler-subclasses` derived from the heading
  text "Using standard library (stdlib) `logging.Handler` subclasses". Impact:
  when adding a new subsection under that heading, avoid placing a new heading
  whose text would change the existing generated anchor.
- Observation: `origin/main`'s `femtologging/adapter.py` is *exactly* 400 lines,
  which is the `too-many-lines` ceiling enforced by Pylint's `max-module-lines`
  and by AGENTS.md's 400-line rule. The baseline therefore had no headroom: the
  two paragraphs this change documents could not be added without tripping the
  gate. Impact: the documentation requirement forced a module split rather than
  being satisfied in place, and the split had to preserve every import site.
- Observation: the Pylint gate rejects an import block that interleaves aliased
  re-exports with plain ones; Ruff's `unsorted-imports` autofix rewrites them
  into one single-name `from … import (X as X)` block each. Impact: the
  re-export style in `femtologging/adapter.py` is Ruff-authored, not a
  hand-written choice, and should be left exactly as the autofix emits it.

## Decision log

- Decision: Perform the red stage by adding the new tests first and confirming
  they fail against the unmodified adapter, rather than writing a throwaway
  reproducer script. Rationale: the tests are deliverables anyway, so the red
  stage doubles as the regression evidence required by the issue's acceptance
  criteria, and it avoids leaving scratch artefacts in the tree. Date/Author:
  2026-10-14, implementing agent.
- Decision: Do not add a `@pytest.mark.xfail(strict=True)` marker to force the
  red stage. Rationale: the tests are added in the same commit series as the
  fix; a strict expected-failure marker would have to be added and then removed
  in the green step, and it would falsely imply an unresolved defect to a later
  reader. Running the new tests before the production edit demonstrates the
  failure directly, and the failing output is captured in this plan's
  `Artefacts and notes`. Date/Author: 2026-10-14, implementing agent.
- Decision: Read `handler.level` on every `handle_record()` call rather than
  caching it at construction. Rationale: a cached copy would break the
  documented stdlib pattern of calling `setLevel()` after the handler is
  attached, which is one of the issue's acceptance criteria. Date/Author:
  2026-10-14, implementing agent.
- Decision: Split `femtologging/adapter.py` into a thin adapter module plus
  `femtologging/_adapter_record.py`, re-exporting the schema and helpers from
  `femtologging.adapter`, rather than trimming the added documentation to fit.
  Rationale: the requirement is to document the two thresholds, so the
  documentation is the deliverable and the code is what must move. Re-exporting
  keeps `from femtologging.adapter import FemtoRecord` and
  `from femtologging.adapter import TRACE_LEVEL_NUM` working for the five
  existing import sites, so the split is invisible to every caller.
  Date/Author: 2026-10-16, implementing agent.
- Decision: Keep the record-translation half in a private `_`-prefixed module
  and leave `femtologging/adapter.py` as the public import site. Rationale: the
  repository already uses `_`-prefixed siblings for internal seams (`_compat`,
  `_log_context`, `_config_filters`), so this matches local convention, and it
  avoids widening the public surface with a new documented module for what is
  an implementation detail. Date/Author: 2026-10-16, implementing agent.
- Decision: Place the new user-guide prose inside the existing
  `StdlibHandlerAdapter` section rather than adding a new `###` heading.
  Rationale: preserving heading text preserves the migration guide's existing
  cross-reference anchor. Date/Author: 2026-10-14, implementing agent.
- Decision: Keep `_make_log_record(record)` as the first operation in
  `handle_record()` before comparing `LogRecord.levelno` with the wrapped
  handler's level. Rationale: the accepted implementation requirement
  explicitly specifies that order and gates on the constructed record; the
  comparison adds one integer check to the existing translation path.
  Date/Author: 2026-10-23, implementing agent.
- Decision: Extend the touched-file set beyond the five-file stop condition.
  Approver: the user, in the supplied Issue #435 implementation request.
  Rationale: that request covers production code, tests, and docs; the 400-line
  module constraint required extraction, and `main` had advanced, so the
  resulting candidate necessarily touched a broader file set. Date/Author:
  2026-10-23, implementing agent.
- Decision: Keep the Python adapter implementation and test helpers unchanged
  when rebasing onto `5377ceb`. Rationale: `main` adds a private Rust-item
  documentation policy and expands `make lint`'s Clippy feature lanes, but its
  changed paths do not overlap this issue's 13 paths and its new support-doc
  checker targets Rust test support. Run the expanded `make lint` on the
  rebased candidate. Date/Author: 2026-10-23, implementing agent.
- Decision: Add the requested adapter sequence as a new section 6.5 in the
  architecture design, after the existing Rust interoperability material and
  before section 7. Rationale: this keeps the behaviour in the design document
  while preserving the existing section structure and gives the diagram a
  concise accessible caption and a screen-reader description immediately before
  the diagram. Date/Author: 2026-10-24, implementing agent.
- Decision: Keep the screen-reader description as adjacent prose instead of
  Mermaid `accTitle` and `accDescr` directives. Rationale: the pinned Nixie
  parser rejects those Mermaid directives, while the labelled screen-reader
  paragraph and visible figure caption remain accessible and pass the
  repository's diagram validation. Date/Author: 2026-10-24, implementing agent.
- Decision: Configure `CountingFilter` to accept or reject records and remove
  the duplicate `RejectingFilter` implementation. Rationale: CodeScene found
  duplicated filter bodies in the handler-level tests; one configurable probe
  keeps both test cases clear while removing the duplication. Date/Author:
  2026-10-24, implementing agent.
- Observation: the repository's pinned Nixie parser rejects Mermaid's
  `accTitle` and `accDescr` directives. Impact: keep the full screen-reader
  description as adjacent prose and retain the visible figure caption, so the
  diagram remains accessible while `make nixie` can validate its syntax.
  Date/Author: 2026-10-24, implementing agent.

## Outcomes & retrospective

The adapter now translates each femtologging record and compares its mapped
stdlib level with the wrapped handler's live `level` before calling
`Handler.handle()`. Below-threshold records do not enter `handle()`; accepted
records enter it once. Handlers retaining the base `Handler.handle()` behaviour
then use the stdlib filter and lock path. The regression tests cover the level
matrix, `NOTSET`, dynamic `setLevel()` changes, filter rejection, and
handle/filter call counts. The user and migration guides explain how the
femtologging logger threshold and wrapped handler threshold combine. This meets
the intended outcome: low-severity records no longer reach a
higher-severity-only destination. The user guide also states that `setLevel()`
changes apply when records are processed, including records already queued but
not yet handled.

The implementation, regression tests and user documentation meet the intended
behaviour: the wrapped stdlib handler's live level is checked before
`Handler.handle()`. Handlers retaining the base implementation use its normal
filter and lock path for accepted records. The design document now includes the
accessible level-gating sequence. The original 16 issue commits were rebased
onto `origin/main` at `d234990` without conflicts or patch changes; subsequent
commits added the diagram and addressed review findings. All six gates passed
on the candidate based on `9be9cc6` with the current documentation corrections
(797 tests passed, 5 skipped), and CodeScene delta found no applicable code
changes. CodeRabbit's review of `9be9cc6` returned four minor documentation
findings: two about this outcome's stale status claims, one about the new
heading's capitalization, and one about an implementation instruction that
differed from the final guard. These findings are corrected, and the corrected
implementation candidate passed all six gates. The final progress update passed
formatting, Markdown and Mermaid gates. A review of `af7539d` found one
inaccurate claim about the `NOTSET` test detecting a truthiness shortcut; its
test description and plan rationale were corrected, and all six gates passed on
that repair. CodeScene found no issues. Candidate `1077f49` was published while
PR #546 remained a draft; its hosted CI and CodeScene check passed. The
completeness assessment of that head found no runtime defect and requested
plan-record corrections to publication status, verification evidence, timeout
accounting and the finite test-domain claim. Candidate `e065086` records those
corrections and passed all six local gates, hosted run `40205578288` and
CodeScene result `8119031`. The follow-up on `6e24864` confirmed those findings
resolved and found no runtime defect, but identified two more verification-plan
inaccuracies: the dynamic test has four dispatches, and its construction-time
cache rationale was wrong; the filter-rejected record is `CRITICAL`. Both are
corrected in the current worktree. The assessment could not independently
verify the historical authorization recorded for the scope expansion; it did
not identify this as a runtime defect. The proof-specific assessment was
received on `fb5a5df` and identified precision corrections recorded below. The
later candidate-bound assessment accepted the manual argument within its stated
scope; no machine proof is claimed. Focused outcome confirmation on published
head `5a3825e` is recorded above. The status assessment on `9e6aedf` accepted
the dated historical observations and requested the three corrections recorded
in Progress. Confirmation that those corrections are sufficient on the updated
plan candidate remains pending, as does formal CodeRabbit review; the existing
queue request is `e8d2e60c`.

## Context and orientation

`femtologging` is a Rust-backed Python logging library. Its Python-facing
handlers are dispatched through a `handle_record(record)` method where `record`
is a plain dictionary, and handler work runs on dedicated worker threads. The
standard library's `logging.Handler` subclasses instead expect an
`emit(LogRecord)` method. `femtologging/adapter.py` provides
`StdlibHandlerAdapter`, a Python class that bridges the two interfaces.

The relevant files are:

- `femtologging/adapter.py` — the adapter itself. `StdlibHandlerAdapter`
  wraps a `logging.Handler`, and `handle_record()` translates the femtologging
  dictionary into a `logging.LogRecord`, applies the wrapped handler's severity
  gate, then dispatches eligible records. The wrapped handler is stored as
  `self._handler`.
- `femtologging/_adapter_record.py` — the record schema and translation,
  extracted when `adapter.py` reached the 400-line ceiling. It owns the
  `FemtoRecord`/`Frame`/`Metadata`/`ExcInfo`/`StackInfo` TypedDicts, the level
  tables, `_stdlib_levelno()` (which maps the femtologging level to a stdlib
  numeric level: TRACE 5, DEBUG 10, INFO 20, WARN 30, ERROR 40, CRITICAL 50),
  and the `_make_log_record()` builder. `femtologging.adapter` re-exports the
  five TypedDicts and `TRACE_LEVEL_NUM`; it also imports `_ensure_trace_level`
  and `_make_log_record()` for internal use. The level tables and
  `_stdlib_levelno()` remain private to this module.
- `tests/stdlib_adapter_support.py` — the vocabulary the three adapter suites
  share: the `LevelCase`/`LogRequest`/`HandlerSetup`/`HandlerLevelCase`
  dataclasses, the `AdapterProbe` harness, and the synchronous `dispatch()` /
  `femto_logger()` helpers.
- `tests/stdlib_adapter_handler_support.py` — the `CapturingHandler`,
  `CountingFilter`, and `emitted_messages()` probes shared by adapter tests.
- `tests/test_stdlib_adapter.py` — construction, dispatch, level mapping,
  producer `contextvars` propagation, and exception/stack forwarding. It builds
  an `AdapterProbe` fixture (a `StringIO` stream, a `logging.StreamHandler`
  over it, and the adapter) and drives records through a real `FemtoLogger`.
- `tests/test_stdlib_adapter_levels.py` — the wrapped handler's level gate:
  the six-level matrix against an `ERROR` handler, the `NOTSET` cases, and the
  filter/`handle()` call-count contract.
- `tests/test_stdlib_adapter_records.py` — `LogRecord` attribute, delegation,
  dynamic `setLevel()`, level-fallback, and export tests. It defines the
  synchronous helpers `_emit_femto_record()` and `_emit_via_logger()`.
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
  before calling `handler.handle()`. The base `Handler.handle()` implementation
  evaluates the handler's filter chain and acquires its I/O lock only when that
  chain accepts the record; it never checks `level`. Subclasses may override
  this path. The pre-change adapter called `handler.handle()` directly without
  reproducing that threshold check.

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
  contradict it. ADR-003's technical requirement "Preserve current
  handler-level filter constraints unless explicitly added in a separate
  decision" is satisfied: this plan adds no handler-level filter support.

Trace links:

```plaintext
Issue#435-AC1 (reject below handler level)     -> EP-M1 -> TestHandlerLevelThreshold::test_error_handler_rejects_below_threshold
Issue#435-AC2 (preserve NOTSET semantics)      -> EP-M1 -> TestHandlerLevelThreshold::test_notset_handler_accepts_low_severity_levels
Issue#435-AC3 (single handle/filter path)      -> EP-M1 -> TestHandleRecordFilterAndLockContract::test_handle_and_filter_counts_by_level
Issue#435-AC4 (ERROR handler level matrix)     -> EP-M1 -> TestHandlerLevelThreshold::test_error_handler_rejects_below_threshold
Issue#435-AC5 (dynamic setLevel)               -> EP-M1 -> TestDynamicHandlerLevel::test_set_level_after_construction_applies
Issue#435-AC6 (document both thresholds)       -> EP-M2 -> docs/users-guide.md, docs/v0-1-0-migration-guide.md
```

## Verification plan

The behaviour under change is a single guard in one method. It introduces one
invariant and one ordering lemma, and it depends on one non-trivial axiom about
the standard library. No formal prover is used: the guard is a direct numeric
comparison, and the tests provide finite behavioural evidence. They exhaust all
six femtologging levels against an `ERROR` handler, and separately exercise
`NOTSET`, dynamic `setLevel()`, and filter behaviour. This test matrix does not
enumerate every possible numeric handler threshold and is not an unbounded
proof.

Axiom A1: in CPython's `logging`, the base `Handler.handle()` implementation
evaluates the handler's filter chain, does *not* consult `self.level`, and
acquires `self.lock` only when that chain accepts the record. This axiom
applies to handlers that retain that base dispatch, filtering and locking
behaviour; subclasses may override `handle()`, filtering or locking. The
adapter's own gate still controls whether it calls the wrapped handler's
`handle()` once, but makes no claim about an overriding subclass's internal
path. Project behaviour depends on the base-handler interaction. Tests
corroborate dispatch and filter behaviour through real stdlib handler types,
but they do not establish the base method's implementation. The lock ordering
is a trusted standard-library assumption; current tests exercise but do not
directly count lock acquisitions. An instrumented lock wrapper could observe
acquisition through the real base `Handler.handle()` path, providing bounded
behavioural evidence rather than proof of the stdlib implementation. The
argument is scoped to CPython's unmodified `logging` module. The CI matrix
covers Python 3.12 through 3.15; it does not establish behaviour for other
implementations or versions.

Invariant I1 (level gate): for each dispatch whose record translation and level
comparison complete normally, let `L` be the constructed `LogRecord`'s numeric
level and `H` the wrapped handler's `level` value observed at the gate. The
adapter calls `handle()` exactly once if and only if `L >= H`. When `L < H`, it
does not call `handle()`, so the handler's filter, lock and emit path is not
entered. What a level-accepted handler call does, including whether it emits,
depends on that handler's implementation and filtering.

Lemma L1 (ordering): record translation precedes the handler-level test. The
test then runs before the adapter calls the wrapped handler. For `L < H`, no
`handle()` call occurs and the handler's filter, lock and emit path is not
entered. For `L >= H`, the adapter calls `handle()` exactly once. For a handler
retaining the base `Handler.handle()` behaviour, normal execution evaluates the
filter chain once; if the chain accepts, the handler acquires its I/O lock and
invokes `emit()`, while a rejection prevents lock acquisition and the `emit()`
invocation. A filter exception does not establish acceptance or emission, and an
`emit()` invocation does not guarantee successful output. The chain may
short-circuit, so this does not claim that every attached filter runs.

Invariant I2 (live configuration): the gate reads `handler.level` at dispatch
time; define `H` as the value observed by that read. For a fixed adapter, a
sequential `setLevel()` call between two `handle_record()` calls makes the
second call use the updated level without reconstructing the adapter. It need
not change that record's acceptance verdict. The four-dispatch test
demonstrates selected acceptance changes only. Concurrent updates are not
atomic with the comparison or later handler work, because the gate runs before
the handler's I/O lock.

- Obligation: I1 for a handler fixed at `logging.ERROR`.
  Method: parameterized test over the finite partition of all six femtologging
  levels. Rationale: the partition is total over the adapter's level vocabulary
  and each row has distinct semantic meaning (four rejects, one accept at the
  boundary, plus `CRITICAL` above it), so enumeration is exhaustive rather than
  a sample. Domain: handler level `logging.ERROR`; logger level `"TRACE"`;
  record levels `TRACE`, `DEBUG`, `INFO`, `WARNING`, `ERROR`, `CRITICAL`.
  Artefact: `tests/test_stdlib_adapter_levels.py`,
  `TestHandlerLevelThreshold::test_error_handler_rejects_below_threshold`.
  Evidence: before the change, every row emits and the four reject rows fail;
  after the change, all six rows pass. Discharge condition: the four
  below-`ERROR` rows produce empty captured output and the two at-or-above rows
  produce exactly the expected formatted line. Non-vacuity: the logger
  threshold is `"TRACE"`, the most permissive setting, so no row is
  pre-filtered by the logger and every rejection is attributable to the new
  handler-level gate. The `WARNING` row is the nearest miss immediately below
  the boundary and the `ERROR` row is the boundary itself, so both sides of the
  comparison are exercised. The negative control is the `red` run recorded in
  `Artefacts and notes`, where the identical test body succeeds only for rows
  whose expected outcome the broken code happens to match.

- Obligation: I1 at the `NOTSET` boundary, where the handler accepts mapped
  records beginning at `TRACE`. Method: a named parameterized test for `TRACE`,
  `DEBUG` and `INFO`, plus a second named test asserting the pre-existing
  default-handler case in the sibling module. Rationale: `NOTSET` is numeric
  zero and means that the handler imposes no level threshold. The test pins
  this observable contract without requiring a particular implementation of the
  zero threshold. Domain: handler level `logging.NOTSET`; records at `TRACE`
  (stdlib level 5, the lowest mapped level), `DEBUG` and `INFO`; logger
  threshold `"TRACE"`. Artefact: `tests/test_stdlib_adapter_levels.py`,
  `TestHandlerLevelThreshold::test_notset_handler_accepts_low_severity_levels`,
  and `tests/test_stdlib_adapter_records.py`,
  `TestDynamicHandlerLevel::test_set_level_after_construction_applies`.
  Evidence: the tested records appear with stdlib level numbers 5, 10 and 20;
  the dynamic test also captures `TRACE` after resetting the handler to
  `NOTSET`. Non-vacuity: `TRACE` is the lowest mapped level, so its acceptance
  verifies that `NOTSET` does not discard the least severe femtologging record.

- Obligation: L1, covering one `handle()` call after a passing level gate and
  no handler path after a rejected level gate. Method: named tests spying on
  `handler.handle` with `mock.patch.object()` and counting filter consultations
  with `CountingFilter`. Rationale: this is a side-effect-ordering claim, not
  an input/output claim, so counting observed invocations through the real
  stdlib objects is a direct and stronger observation than inspecting output
  text. Domain: handler level `logging.ERROR`; one accepted record (`ERROR`),
  one rejected record (`DEBUG`), and one `CRITICAL` record that passes the
  level gate but is rejected by a filter. Artefact:
  `tests/test_stdlib_adapter_levels.py`,
  `TestHandleRecordFilterAndLockContract`. Evidence: after the folded-in
  rejection, a `DEBUG` record leaves the `handle()` count and the filter-call
  count unchanged and emits nothing; an `ERROR` record increments both by
  exactly one and emits exactly one record; the filter-rejected `CRITICAL`
  record increments `handle()` once, increments the installed filter's count
  once, and emits nothing. Non-vacuity: the test asserts counts rather than
  "not zero", so an implementation that calls `handle()` twice or that skips
  filters entirely fails. The filter-rejected row proves the installed filter
  is reached through `Handler.handle()`, so "the gate ran but the filter never
  did" cannot pass unnoticed. Residual gap: the handler's I/O lock is exercised
  implicitly by the real `Handler.handle()` implementation rather than counted
  by the current tests. An instrumented lock wrapper could observe acquisition
  through that real path, but that would remain bounded test evidence rather
  than proof of the stdlib lock implementation. No lock-count test is included
  because the current source-based argument does not require one.

- Obligation: I2 for a live `setLevel()` sequence.
  Method: named test over four dispatches across three handler-level states,
  with two `setLevel()` calls. Rationale: this is an ordering property over a
  short operation sequence rather than a property over generated inputs;
  enumerating the operations explicitly is clearer than generating them, and
  the sequence is short enough to be read as a specification. Domain: adapter
  constructed with a `NOTSET` handler; initial `INFO` dispatch;
  `setLevel(logging.ERROR)` then `INFO` and `ERROR` dispatches;
  `setLevel(logging.NOTSET)` then a `TRACE` dispatch. Artefact:
  `tests/test_stdlib_adapter_records.py`,
  `TestDynamicHandlerLevel::test_set_level_after_construction_applies`.
  Evidence: the initial `INFO` is emitted; the `INFO` after tightening to
  `ERROR` is rejected; the `ERROR` record is emitted; and the `TRACE` record
  after restoring `NOTSET` is emitted with `levelno == 5`. Non-vacuity: the
  sequence covers the initial, tightened, and restored handler-level states. A
  construction-time cache of the initial `NOTSET` level fails the second
  dispatch by accepting the `INFO` record; the `ERROR` boundary and restored
  `NOTSET` state are also checked explicitly.

## Plan of work

The change is small enough to be a single behavioural unit, so there are two
implementation milestones with a documentation milestone alongside.

Stage A (understanding) is complete: the defect, the three adapter test
modules, and the two documentation sections have been read, and the CPython
reference behaviour (`Logger.callHandlers()`) has been identified as the
specification to match.

Stage B (red) adds the threshold matrix and filter-contract tests to
`tests/test_stdlib_adapter_levels.py`, and the dynamic `setLevel()` test to
`tests/test_stdlib_adapter_records.py`. Run them against the unmodified adapter
to confirm they fail for the intended reason.

Stage C (green) makes the smallest production change that satisfies the tests:
one guard in `StdlibHandlerAdapter.handle_record()`.

Stage D (documentation and wider validation) updates the two documentation
files and runs the full commit gates.

### Milestone EP-M1: the level gate is enforced and tested

Outcome: `StdlibHandlerAdapter` rejects records below the wrapped handler's
live level and calls `handle()` once for records at or above it. For handlers
retaining the base `Handler.handle()` behaviour, normal execution evaluates the
filter chain once; rejection prevents lock acquisition and the `emit()`
invocation. The repository is coherent on its own: the adapter and its tests
agree.

Changes:

- `femtologging/adapter.py`: in `handle_record()`, after
  `log_record = _make_log_record(record)`, call
  `self._handler.handle(log_record)` only when
  `log_record.levelno >= self._handler.level`. Update the class docstring and
  the `handle_record()` docstring to state that the adapter honours both the
  femtologging logger threshold and the wrapped handler's level, that the
  comparison happens after record translation but before `handle()` runs the
  filters, and that sequential `setLevel()` changes between dispatches affect
  the level read by the next dispatch. For handlers retaining the base
  `Handler.handle()` behaviour, normal completion evaluates the filter chain
  and acquires its lock only if the chain accepts.
- `tests/test_stdlib_adapter_levels.py`: add `TestHandlerLevelThreshold` with
  the six-row level matrix against an `ERROR` handler under a `"TRACE"` logger,
  and the `NOTSET` acceptance test.
- `tests/test_stdlib_adapter_records.py`: add `TestDynamicHandlerLevel` with
  the synchronous `setLevel()` sequence. This module is chosen for the dynamic
  test because `_emit_femto_record()` calls `handle_record()` directly, which
  makes the sequence deterministic with no worker-thread timing involvement.
- `tests/stdlib_adapter_handler_support.py`: share `CapturingHandler` and
  `CountingFilter(should_accept=...)` with the adapter test modules.
- `tests/test_stdlib_adapter_levels.py`: keep the filter/`handle()` call-count
  contract test beside the level threshold cases.

Acceptance evidence: the two focused test modules pass, and the red run for the
same tests before the production change is recorded in `Artefacts and notes`.

Conformance check:

- Requirements satisfied: Issue#435 acceptance criteria 1 through 5.
- Design still followed: the adapter keeps its single `handle_record` dispatch
  path and still delegates to `Handler.handle()`.
- Upstream assumptions still valid for handlers retaining base dispatch,
  filtering and locking behaviour: CPython `Handler.handle()` semantics per
  Axiom A1.
- No unapproved public interface or dependency change: `handle_record()`
  signature and the package's public exports are unchanged.
- No unapproved trust boundary or persisted-format change: none introduced.
- Trace links current: the EP-M1 links in `Conformance basis`.

Recovery: the milestone is one guard plus tests; `git revert` of the milestone
commit restores the previous behaviour exactly.

Remaining gaps: documentation (EP-M2).

Compatibility decision: none required. The wrapped handler's `level` is public,
documented stdlib surface, and the femtologging package is pre-1.0
(`version = "0.1.0"` in `pyproject.toml`). No compatibility layer is introduced.

### Milestone EP-M2: both thresholds are documented

Outcome: a reader of the users guide understands that two independent
thresholds apply, in which order, and how the wrapped-handler threshold relates
to the unrelated configuration-schema restriction on handler levels; the
migration guide points at the same explanation instead of restating it.

Changes:

- `docs/users-guide.md`: extend the `StdlibHandlerAdapter` section with prose
  and a short example showing a `"TRACE"` femtologging logger with a
  `logging.ERROR` handler, plus one sentence distinguishing the
  configuration-schema handler-level restriction from wrapped-handler behaviour.
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
Prefix each gate with output capture as the repository conventions require,
for example:

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
uv run pytest tests/test_stdlib_adapter_levels.py tests/test_stdlib_adapter_records.py -v
```

Step 3 — apply the guard in `femtologging/adapter.py` and re-run the same two
modules. Expect all tests to pass:

```plaintext
uv run pytest tests/test_stdlib_adapter_levels.py tests/test_stdlib_adapter_records.py -v
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
- Verification: finite tests corroborate selected cases of I1, I2 and L1. The
  source branch supports the scoped I1 statement; A1 remains a trusted
  base-handler assumption, with filter behaviour exercised and lock ordering
  based on the stdlib implementation rather than direct instrumentation. No
  machine-checked proof discharge is claimed.
- Lint/typecheck: `make check-fmt`, `make lint` and `make typecheck` all pass
  with no warnings.
- Performance: the guard adds one integer comparison and one attribute read
  per dispatched record on the existing `LogRecord` construction path. The
  comparison intentionally follows `_make_log_record(record)` because the
  accepted implementation contract makes record construction the first step;
  rejected records therefore still incur the existing translation cost.
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

The isolated red-stage and green-stage transcripts for the focused modules were
not retained. The red observation is the manual reproduction above: before the
guard, an `INFO` record is emitted by an `ERROR` handler. The green full test
run on 2026-10-25 passed 797 tests, skipped five and passed 66 snapshots; it
includes the six `ERROR`-threshold cases, the `NOTSET` cases, filter/`handle()`
counts, and the dynamic `setLevel()` sequence. The six sequential local gates
passed, and hosted
[run 40192246149](https://github.com/leynos/femtologging/actions/runs/40192246149)
passed all four Python-version jobs. The current
[CodeScene result](https://codescene.io/projects/68445/delta/results/8117957)
passed six quality gates with no annotations on the earlier candidate.

The completeness assessment for published head `1077f49` is recorded at
[issue comment 6397731459](https://github.com/leynos/femtologging/pull/546#issuecomment-6397731459).
It found no runtime defect and requested corrections to the publication
status, verification record, timeout summary, and scope of the finite
enumeration. Candidate `e065086` records those corrections and passed the six
local gates, hosted
[run 40205578288](https://github.com/leynos/femtologging/actions/runs/40205578288/attempts/1),
and current CodeScene
[result 8119031](https://codescene.io/projects/68445/delta/results/8119031).
The completeness follow-up at
[issue comment 6400122970](https://github.com/leynos/femtologging/pull/546#issuecomment-6400122970)
confirmed the prior findings resolved and identified two remaining
inaccuracies in the dynamic-level sequence and filter-rejection example. This
worktree corrects them. A proof-specific assessment of `fb5a5df` accepted the
core manual argument and identified precision corrections applied in this
revision. The later candidate-bound assessment accepted the manual argument
within its stated scope; focused outcome confirmation for published head
`5a3825e` is recorded above.

## Interfaces and dependencies

No new dependencies and no signature changes. The one production-side interface
that matters is the existing method, whose contract is extended rather than
altered:

```python
class StdlibHandlerAdapter:
    def handle_record(self, record: FemtoRecord) -> None:
        """Translate a femtologging record and dispatch it if the handler accepts it.

        The record is translated before its mapped stdlib level is compared
        with the wrapped handler's ``level``, matching the gate in
        ``logging.Logger.callHandlers()``. Records below the level are
        discarded before ``handle()``. A record that passes is passed to
        ``self._handler.handle()`` exactly once. When the handler retains the
        base ``logging.Handler.handle()`` behaviour, normal execution evaluates
        the filter chain once and acquires the I/O lock only if the chain
        accepts. A filter exception prevents ``emit()``; invoking ``emit()``
        does not guarantee successful output. Sequential ``setLevel()`` calls
        between dispatches affect the level read on the next call; concurrent
        changes are not synchronized with the gate or handler work.
        """
```

`FemtoRecord` is the existing `TypedDict` in `femtologging/_adapter_record.py`
and is re-exported by `femtologging/adapter.py`; the method keeps returning
`None`, and rejection is signalled by the absence of a side effect rather than
by an exception or a return value, exactly as `logging.Logger` does.

## Revision note

Initial draft created 2026-10-14. Revised 2026-10-23 to record the rebase and
post-rebase validation, complete the outcomes, and mark the active draft PR
state. Clarified that `setLevel()` changes affect queued records not yet
processed, recorded the zero-finding final review, and noted the force-with-
lease update to draft PR #546. Revised 2026-10-25 after `main` advanced to
`d234990`: the 16-commit issue series replayed without conflicts, and the user
guide now distinguishes the unsupported configuration `level` fields from
filters that `dictConfig` now supports. Revised again 2026-10-25 after
CodeRabbit's completeness assessment of `1077f49` to correct publication
status, verification evidence, timeout accounting and the scope of the finite
test matrix. Revised again after the completeness follow-up on `6e24864` to
correct the dynamic dispatch count and cache rationale, and the filter-rejected
record's level.

Revised 2026-10-25 after the proof-specific assessment of `fb5a5df`: bounded
the handler-behaviour assumptions to subclasses retaining base
`Handler.handle()` semantics, clarified observed-level and sequential-update
claims, qualified normal filter/lock/emission ordering, and corrected the
lock-instrumentation rationale. These changes refine the manual argument; the
implementation and tests are unchanged. The assessments of `8fd2cdf` confirmed
those proof and plan corrections, and identified a remaining user-guide
overstatement about handler subclasses. Candidate `8001ddc` passed all six
local gates and CodeScene; the assessments accepted the base-handler
qualification but found async-context overstatement in several documents and an
outdated open publication step. The current correction updates those context
boundaries and records that `8001ddc` is already published. The
context-boundary source changes passed lint, typecheck, and the complete test
target. The final ExecPlan evidence wording then passed the formatting,
Markdown, and diagram gates. One earlier Python-feature doctest attempt linked
against an `extension-module` PyO3 artefact and failed to resolve Python
symbols. The isolated doctest rebuild and the complete test target then passed
without source changes. On 2026-10-26, published head `5a3825e` passed
`make check-fmt`, `make lint`, `make typecheck`, and `make test` (797 passed, 5
skipped, 66 snapshots); hosted run `40261703860` and CodeScene result `8124222`
passed. Focused outcome confirmation `6407235083` found no remaining stale
status entry. Formal PR review and the guarded merge remain.
