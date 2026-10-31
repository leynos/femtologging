# Release the GIL around Python-exposed file-handler operations that may block

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

Related: [Issue #441](https://github.com/leynos/femtologging/issues/441)

## Purpose / big picture

Python can construct `FemtoFileHandler` with `policy="block"` or
`policy="timeout:N"`, then call its public `handle()` method directly. Those
methods enter potentially blocking Crossbeam channel operations while PyO3
still holds the GIL, so a slow or stalled file worker can stall the entire
interpreter rather than just the calling thread.

After this work, every Python-exposed file-handler wait releases the
interpreter lock before it waits, so other Python threads keep running while
one thread is parked on back pressure.

Observable outcomes:

- `handler.handle(...)` under `policy="block"` no longer starves a second
  Python thread.
- `handler.flush()` and `handler.close()` release the lock across their waits.
- While detached `close()` waits with its exclusive PyO3 borrow, concurrent
  `handle()`, `flush()`, and a second `close()` call on the same object raise
  `RuntimeError`.
- The rotating and timed-rotating wrappers inherit the same contract.
- Python tests, isolated in subprocesses, demonstrate a second thread making
  progress while a direct `handle()` call is blocked.

## Constraints

- Python-visible signatures must not change; the `.pyi` stubs stay as they are.
- Argument parsing and Python exception construction stay on the GIL-owning
  side of the boundary.
- The Rust `flush()`/`close()` implementations, their one-second deadlines, and
  `Drop` are unchanged.
- `--no-default-features` and `--cfg loom` builds must keep compiling. Loom
  compiles the bindings but does not invoke Python methods or model the GIL; no
  cfg guard is needed around `Python::detach` in these bindings.
- `policy="block"` stays available to Python; its shutdown semantics are
  documented rather than changed.

## Tolerances (exception triggers)

- If detaching changes observable Python behaviour beyond the PyO3 borrow
  errors during an active exclusive `close()` call, stop and re-plan. The
  borrow errors must be documented and tested.
- If the Loom lane cannot be kept green without weakening a model, stop and
  raise it rather than weakening the model.

## Risks

- **Deadlock via re-entrancy.** `py.detach` releases the lock around a wait
  whose worker might itself need the GIL. The file worker formats through a Rust
  `FemtoFormatter`; the Python-callable formatter is adapted at build time, so
  the worker can acquire the GIL normally. Detaching removes a lock the caller
  held, which can only reduce deadlock risk, not raise it.
- **`Send` bounds.** `py.detach` requires the closure and its return type to be
  `Ungil` (i.e. `Send`). `HandlerError` must be checked for `Send`; if it is
  not, the closure maps it to a `Send` representation.
- **Test flakiness.** The regression test asserts a bounded number of watchdog
  progress records land inside the blocked call's own window. Subprocess
  isolation, generous in-child waits and a low threshold keep it stable.

## Conformance basis

Issue [#441](https://github.com/leynos/femtologging/issues/441) and source
finding `3e0c8fe12a0081918a117d28e165c10e` define the implementation scope:
Python-exposed file-handler waits must release the GIL, and the block and
timeout policies must be covered by observable concurrency tests. The issue's
acceptance items map to Tasks 1–4 below and the verification obligations in
this plan. There is no separate Terms of Reference, technical design, or ADR
for this issue; no new public signature or dependency is introduced.

## Verification plan

- **Boundary invariant:** parsing and Python exception creation remain attached
  to the interpreter; only Rust-owned values and operations cross `py.detach`.
  PyO3 0.30.1 documents that the closure and return value must implement
  `Ungil`; the Rust compiler checks this bound. The methods are covered by
  `make typecheck`, every Clippy feature lane, and the Loom build included in
  `make test`.
- **Interpreter-progress invariant:** a full capacity-one queue parks the
  producer in `send` or `send_timeout` while another Python thread records
  progress inside that call's window. The subprocess tests exercise all three
  file-handler builders; the flush and close scenarios exercise their
  acknowledgement and shutdown waits. `make test` builds the extension and runs
  these tests. The negative control reintroduced the original GIL-holding call
  and failed exactly the three direct file-handler tests, so the oracle rejects
  the intended defect.
- **Backpressure and shutdown contract:** the existing Rust implementations,
  one-second flush and close deadlines, and `Drop` remain unchanged. The
  scenarios check timeout failure, accepted-record output, `flush() == False`
  after its deadline, and the `RuntimeError` from a concurrent Python `close()`
  while a blocked call retains its shared PyO3 borrow. They also check that
  `handle()`, `flush()`, and a second `close()` fail with `RuntimeError` while
  detached `close()` holds the exclusive PyO3 borrow.
- **External interface assumptions:** the version-matched PyO3 0.30.1 API
  documents `Python::detach` and its `Ungil` bound at
  <https://docs.rs/pyo3/0.30.1/pyo3/marker/struct.Python.html#method.detach>.
  Crossbeam Channel 0.5.15 documents that `Sender::send` blocks until send or
  disconnection, while `send_timeout` additionally returns when its duration
  expires at
  <https://docs.rs/crossbeam-channel/0.5.15/crossbeam_channel/struct.Sender.html>.
  These third-party semantics are treated as assumptions; the tests exercise
  the repository's real bindings and worker queue. No formal proof or model
  changes are introduced; Loom continues to cover the Rust channel lifecycle,
  while the Python subprocess tests cover the GIL boundary.
- **Documentation invariant:** the user and developer contract stays aligned
  with the implementation and builder outputs. `make markdownlint` checks
  prose, spelling, and links; `make nixie` checks diagrams.

## Progress

- [x] Reconnaissance, builder audit, and source-boundary review completed.
- [x] Tasks 1–4 implemented: shared, rotating, and timed-rotating Python
      bindings detach around waits; builders need no changes; Python
      subprocess regressions and user/developer documentation added.
- [x] The negative control reintroduced the original file-handler `handle()`
      defect and failed exactly the three file-handler tests that exercise it.
- [x] Previously rebased onto `origin/main` at `5377ceb`, including PyO3 0.30.1
      and the expanded Clippy feature matrix; resolved the three handler-file
      conflicts.
- [x] The first complete gate sequence on HEAD
      `a994236507ae11a3280018143f8b3b70323b4892` passed
      `make check-fmt`, `make lint`, `make typecheck`, `make test`,
      `make markdownlint`, and `make nixie`. The test target reported 763
      passed, 5 skipped, 1 warning, and 66 snapshots passed; all nine GIL
      regressions passed.
- [x] The first `coderabbit review --agent` completed with 12
      findings. Triage identified follow-ups: document and test PyO3 borrow
      errors during detached `close()`, derive outer timeouts from both nested
      subprocess budgets, and clean stale plan and prose details.
- [x] Addressed the review follow-ups: expanded flush/close and borrow-error
      scenarios across all three wrappers, derived nested-test deadlines from
      their subprocess budgets, and reconciled the plan and user-guide prose.
- [x] Completed the first-review follow-ups and reran all six deterministic
      gates. `make test` reported 775 passed, 5 skipped, 1 warning, and all 21
      GIL regression cases passed.
- [x] The second CodeRabbit review completed with three findings. The two
      support-helper docstrings now have structured parameters and documented
      result semantics, and overflow-policy docs limit the no-wait claim to
      `handle()` enqueues.
- [x] Resolved Ruff's docstring follow-ups by documenting the runner's
      no-value result in prose, as Ruff rejects a `Returns` section for an
      implicit `None`; removed the renderer's implicit `KeyError` section.
- [x] The third CodeRabbit review identified sentence-case headings and an
      ineffective close instruction. The headings are normalized, and the
      shutdown guidance now says to restore worker progress and let the
      blocked call finish before closing.
- [x] Reran all six deterministic gates after the third-review fixes.
      `make test` reported 777 passed, 5 skipped, 1 warning, and all 21 GIL
      regression cases passed; Markdownlint reported 0 errors and Nixie
      validated every diagram.
- [x] Rebased the 13-commit issue series from `5377ceb` onto the fetched
      `origin/main` at `d234990`. The `range-diff` retained all 13 commits; the
      only conflict was in `docs/users-guide.md`. The resolution preserves the
      newer formatter guidance while carrying forward the GIL contract and
      blocking-policy qualification.
- [x] Reran all six deterministic gates on the rebased candidate rooted at
      `723fb02`, including the reapplied review follow-ups. `make test` reported
      804 passed, 5 skipped, 1 warning, and 66 snapshots passed; Markdownlint
      checked 61 files with 0 errors, and Nixie validated every diagram. The
      source and test files have not changed since this complete run.
- [x] Completed the post-cooldown CodeRabbit review against the rebased
      candidate. It reviewed 12 files and raised one valid minor documentation
      concern: a blocked file-handler worker can leave the logger's separate
      bounded queue unable to drain, so `logger.log()` may still drop records.
      The user guide now distinguishes file-handler and logger queue drops.
- [x] Reran the docs-only gates after the documentation repair: `make fmt`,
      `make check-fmt`, `make markdownlint`, and `make nixie` passed. The
      CodeRabbit follow-up reviewed all 12 changed files and reported 0
      findings.
- [x] Pushed the original implementation and created draft PR #550 with
      `Closes #441` and the Lody session reference.
- [x] The final CodeRabbit review identified missing coverage for a second
      `close()` call during detached close. Added the case for all three
      builders; the complete six-gate run passed with 807 tests passed, 5
      skipped, 1 warning, and 66 snapshots passed.
- [x] Committed and pushed the close-on-close review follow-up as `c6ebda9`.
- [x] The review of `c6ebda9` found two scope overstatements: the user guide's
      GIL guarantee needed to name explicit Python calls, and the developer
      guide needed to acknowledge other blocking handler families. Both prose
      findings were corrected and pushed as `da7d568`; formatting, Markdownlint
      and Nixie passed, the draft PR body was refreshed, and CodeRabbit reported
      no findings on the updated head.
- [x] Rebased the branch from `33858df` onto the fetched `origin/main` at
      `187aea7`. All 17 issue commits plus the CodeScene-directive commit
      replayed byte-identically; only `docs/developers-guide.md` changed on
      both sides, and the merge keeps both the target's Dependabot and Rust
      build-standard sections and this branch's GIL sections. Every
      target-only path remains byte-identical to the target.
- [x] Rerun the six deterministic gates on the rebased candidate and record the
      results here. On `ef5bc04` plus the test-isolation and CodeScene fixes,
      `make check-fmt` (1 s), `make lint` (2 496 s, including all eight Rust
      Clippy lanes, the five-stage Python pipeline and the Whitaker Dylint
      pass), `make typecheck` (200 s) and `make test` (5 962 s) all exited 0.
      `make test` reported 855 passed, 5 skipped and 66 snapshots passed.
      `make markdownlint` and `make nixie` passed on the final head. The long
      durations are the shared compile queue, not failing work; see
      "Host compile admission and the build-standard contract tests".
- [x] Committed the host-isolation fix for the build-standard contract tests as
      `0725590` and the timed-rotating CodeScene directive as `3c92306`.

## Negative control (evidence that the tests detect the defect)

The suite was run against a deliberately reintroduced defect. Only `py_handle`
in `handlers/file/mod.rs` was changed back to calling
`FemtoHandlerTrait::handle` directly, holding the GIL; `py_flush`, `py_close`,
and both rotating bindings were left fixed. The extension was rebuilt
(`make build`, exit 0) and the suite re-run:

| Test                                                      | Result | Child's own diagnosis                                                                       |
| --------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------- |
| `test_blocking_handle_releases_the_gil[file]`             | FAIL   | child never finished within the 20s budget                                                  |
| `test_timed_handle_releases_the_gil[file]`                | FAIL   | `recorded only 2 progress record(s) during the 2.006s wait in handle (expected at least 5)` |
| `test_close_while_handle_is_blocked_raises_runtime_error` | FAIL   | child never finished within the 20s budget                                                  |
| `...[rotating]`, `...[timed-rotating]`                    | pass   | those bindings were left fixed                                                              |
| `flush`, `close`                                          | pass   | those bindings were left fixed                                                              |

The three failures are exactly the three tests that exercise `handle()` on the
file handler. The 2-record result against a 5-record bar, during a 2.006 s wait
where a released wait yields hundreds, is the discriminating signal the verdict
was designed around. The suite is therefore regression evidence rather than a
description of current behaviour.

The fixed source was restored afterwards and the working tree confirmed
identical to the committed revision before rebuilding.

A second, cheaper control was run without any rebuild: the scenario harness was
pointed at `ctypes.PyDLL.usleep` (holds the GIL across the callee, like a
blocked Rust binding) and `ctypes.CDLL.usleep` (releases it). The detector
rejected the first and accepted the second. A pure-bytecode busy-wait would
*not* have been a valid stand-in for the defect, because CPython's eval breaker
drops the GIL every few milliseconds; only a call that blocks below the
interpreter reproduces the freeze.

## Surprises & discoveries

- `handlers/rotating/core.rs` and `handlers/timed_rotating/core.rs` both
  `delegate!` `flush`/`close`/`handle` to an inner `FemtoFileHandler`, so the
  wrappers are thin and the detach belongs in the wrapper `#[pymethods]`, not
  the core.
- `handlers/mod.rs` is **not** Python-gated: `pyo3::create_exception!` and the
  `#[pyclass]` attributes on `FemtoFileHandler` compile without the `python`
  feature, because `pyo3` is an unconditional dependency with `auto-initialize`
  and `macros`. The `python` feature only gates `extension-module` linking and
  a handful of `#[cfg(feature = "python")]` re-exports. This matters because it
  means the `#[pymethods]` blocks always compile.
- The `--cfg loom` lane builds `--no-default-features`, so `feature = "python"`
  is off but the `#[pymethods]` code still compiles. **Correction to the
  original reading:** no `cfg(not(loom))` guard is needed here.
  `logger/worker.rs` needs one because it *attaches* to the interpreter from a
  thread that does not have a token; these methods only consume a `Python<'_>`
  token PyO3 already handed them. Nothing in the bindings calls
  `Python::attach`.
- `Python::detach` does **not** release the pyclass borrow flag. A blocked
  `handle()` therefore holds its shared borrow for the whole wait, and a
  concurrent `close()` fails its exclusive borrow with `RuntimeError`. That is
  safe rather than a defect, and Task 3 pins it as a contract.
- A Loom model cannot observe the GIL at all: the model replaces both the
  channel and the thread, so there is no interpreter holding a lock to release.
  The regression evidence has to come from Python, which is why Task 3 spawns
  subprocesses.
- The timed-rotating Python wrapper maps enqueue failures to `ValueError`,
  whereas the file and size-rotating wrappers use `RuntimeError`. The GIL
  change keeps these existing exception mappings on the attached side of
  `detach`; the timeout documentation and parametrized tests record the current
  distinction instead of changing Python-visible behaviour.

## Rebase onto a moved `origin/main`

While the gates were running, `origin/main` advanced six commits, including a
pyo3 upgrade and a new lint. The branch had to be rebased before it could be
reviewed, let alone merged.

| Upstream change                                                         | Effect on this branch                                                                             |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `pyo3` 0.29.3 → 0.30.1 (Dependabot #542)                                | None in source. `Python::detach` keeps the same signature and `Ungil` bounds in 0.30.1.           |
| Deny `missing_docs_in_private_items` (#422, #453)                       | Every item on this branch already carries a doc comment, including the three `py_handle` methods. |
| Clippy feature lanes 5 → 8 (`extension-module`, `test-util`, `default`) | Widens the matrix each lane is checked under; no source change.                                   |

The three conflicted files were `handlers/file/mod.rs`,
`handlers/rotating/python.rs` and `handlers/timed_rotating/python.rs`; upstream
had attached doc comments to the very methods this branch rewrote. Both sides
only ever added prose, so each hunk was resolved by keeping upstream's
description and appending this branch's GIL contract to it, then re-applying
the three-method change. Nothing was dropped from either side.

Two artefacts came out of the resolution and were removed in their own commit:
the merge tool appended a `// weave: run 'weave explain ...'` hint to the end
of each resolved file, and doubled two blank lines in `handlers/file/mod.rs`.
`rust_extension/src/logger/mod.rs` carries the same hint on `origin/main`; it
is upstream's, and untouched here.

At that point, `git rev-list --left-right --count origin/main...HEAD` reported
`0 9`, so the branch was a strict fast-forward from main, and the diff against
main was the intended ten files.

## Rebase onto `origin/main` at `d234990`

Before publication, the fetched target advanced to `d234990`
(`Add contextual handler filters and formatters`). The previous exclusive base
remained `5377ceb`: the earlier rebase receipt and reflog show that commit was
`origin/main` when the issue series began, and the branch contains no merge
commits. All 13 later commits are issue-specific.

The candidate's tracked, unstaged changes and untracked test scenario were
preserved in a verified recovery snapshot and stash before replay. The replay
used Git's built-in `zdiff3` merge driver because the repository's tracked
attributes select Weave for Rust files and this unattended rebase did not
request Weave dogfooding. All 13 commits replayed. `git range-diff` shows an
expected change to the documentation commit: it now retains the newer
formatter-builder and percent-style guidance from the target. The only new
conflict was in `docs/users-guide.md`; its resolution keeps those upstream
changes and adds the file-policy qualification plus the GIL contract. The
working changes then reapplied without conflicts. The rebased branch reports
`0 13` against `origin/main` at `d234990`.

The earlier six-gate run and CodeRabbit review apply to `a994236`, before this
rebase. The post-cooldown reviews and follow-ups also completed on the rebased
candidate. The close-on-close test follow-up passed all six deterministic gates
and was pushed. The final two documentation-scope findings were corrected and
pushed in `da7d568`; the docs gates passed and CodeRabbit reported no findings
on that head.

The earlier pre-rebase gate results did not apply to this dependency and lint
matrix. The complete gate run on the rebased candidate is recorded in
`Progress`.

## Rebase onto `origin/main` at `187aea7`

The PR target advanced again after the implementation was complete. The fetched
target was `187aea7`; the exclusive base was `33858df`, the commit on
`origin/main` from which the issue branch was cut. The branch contains no merge
commits, `git cherry` marks every commit as absent upstream, and the series
held 17 issue commits before the replay, so no inherited parent work was in
range.

The target's five new commits were dependency and build-standard changes:

| Upstream change                                  | Effect on this branch                                                                                                                                         |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Repin `install-whitaker` (#574)                  | Workflow-only; the Whitaker Dylint lane runs unchanged from this branch.                                                                                      |
| `setup-uv` action bump (#557)                    | Workflow-only.                                                                                                                                                |
| Rust build standard (#577)                       | Adds `.cargo/config.toml`, `rust_extension/Cargo.toml` settings, Makefile and workflow changes, and a contract test. This branch touches none of those paths. |
| Dependabot moves to uv and groups maturin (#578) | Adds a Dependabot contract test and a developer-guide grouping section, which the merge retains.                                                              |
| Minor-and-patch group bump (#579)                | `uv.lock` only; the target version is inherited.                                                                                                              |

`merge=weave` was selected for Rust and Python files only by the user-wide
attributes file at `~/.config/git/attributes`; the repository tracks no
`.gitattributes` and has no `.git/info/attributes` rule. Under the unattended
rebase policy the replay therefore used Git's built-in merge machinery with
`zdiff3` and `-c core.attributesFile=/dev/null`, and no Weave driver
participated.

All 18 commits replayed, and `git range-diff` reports every commit unchanged.
No conflict was raised. The only path changed on both sides is
`docs/developers-guide.md`: the target-only paths (`.cargo/config.toml`, the
workflows, `Makefile`, `rust_extension/Cargo.toml`, the dependency contract
tests, and `uv.lock`) are byte-identical to `187aea7`, and the merged guide
keeps the target's `Dependabot grouping`, `Rust build standard`,
`Development build footprint`, and `Cranelift` sections alongside
`Releasing the GIL at the Python boundary` and `Testing the boundary`. The
branch now reports 18 commits ahead of `origin/main`.

The earlier gate evidence applies to the pre-rebase head; the gate run for this
candidate is recorded in `Progress`.

## Host compile admission and the build-standard contract tests

The first `make test` on the rebased candidate failed three cases in
`tests/test_build_standard_contract.py`, a file the rebase inherited unchanged
from `187aea7`. Two failed with `Timeout (>30.0s) from pytest-timeout` and one
with a `cargo build failed` message in which sccache refused incremental
compilation.

`_compile_fixture` copies the repository's `.cargo/config.toml` into a one-file
crate and removes every wrapper variable from the environment, which assumes
the repository is the only source of Cargo configuration. It is not: this host's
`~/.cargo/config.toml` sets

```toml
[build]
rustc-wrapper = "/home/leynos/.local/bin/build-limits-rustc"
rustdoc = "/home/leynos/.local/bin/build-limits-rustdoc"
```

and no environment variable overrides that. The fixture's three cargo builds
therefore entered the shared compile-admission pool, where they queued behind
other builds for longer than the suite-wide 30-second timeout, and the
incremental-override case failed because the admission wrapper refuses
incremental compilation for cache reasons.

The fixtures were not weakened. `CARGO_HOME` now points at an empty temporary
directory, so the fixture sees only the repository's own configuration, which
is the boundary the test exists to pin. The crate has no dependencies, so the
relocated home needs no registry cache, and the three cases pass in about a
second. Neither the repository configuration nor the assertions changed, and
the failure was host-specific rather than caused by the GIL change.

## Decision log

- **Detach in the `#[pymethods]` shims, not the core.** The core
  `FemtoFileHandler::flush`/`close` are also called from Rust and from `Drop`;
  detaching is a Python-boundary concern, so it stays in the binding.
- **Detach for every policy, not just the blocking ones.** `Drop` uses
  `try_send` and does not wait, but branching would leave `drop` on a different
  code path for no benefit. One uniform boundary is simpler to audit.
- **`policy="block"` stays.** Removing it would be a breaking change and the
  issue asks for a decision plus documentation, not removal. Its unbounded wait
  and the fact that `close()` does not cancel it are now documented in the user
  guide, and callers who need a wait that ends on its own are pointed at
  `"timeout:N"`.
- **Preserve handler-specific exception mappings.** The detached closure returns
  the Rust error, and each binding constructs its existing Python exception
  after the GIL is reacquired. In particular, the timed-rotating wrapper's
  `ValueError` mapping is not normalized as part of this GIL-only change.
- **Keep PyO3 borrow errors during detached close explicit.** `close()` holds
  its exclusive object borrow while waiting without the GIL. Concurrent
  `handle()`, `flush()`, or `close()` calls on that object raise
  `RuntimeError`; the user guide and subprocess tests pin this behaviour.
- **Assert the *shape* of the wait, not a wall-clock threshold.** The first
  draft asserted only that a progress event fired before the blocked thread was
  joined. That passes for a GIL-holding binding whenever the wait ends on its
  own deadline (`"timeout:N"`, `close()`), so the bounded scenarios now count
  watchdog records that land strictly inside the call's own window.

## Follow-up work (audited, deliberately out of scope)

Issue #441 is scoped to the **file** handlers. The same boundary gap exists on
the other handler families, and the audit confirms it is real rather than
theoretical:

- `rust_extension/src/http_handler/handler.rs` — `py_flush` detaches (the
  precedent this change followed), but `py_handle` and `py_close` do not. A
  flush timeout defaults to the handler's `write_timeout` (30 seconds), so a
  `close()` there can hold the interpreter lock for half a minute.
- `rust_extension/src/socket_handler/handler.rs` — `handle`, `flush` and
  `close` all still run with the lock held.
- `rust_extension/src/stream_handler.rs` — same, all three.

They were left unchanged because the issue's acceptance criteria name the file
handlers and the builder audit confirmed no builder work is needed; widening
the change would make the diff harder to review and the regression evidence
harder to scope. Tracking these as a separate issue is the right size.

## Superseded gate interruptions and outcomes

Earlier gate attempts were delayed or timed out while Cargo waited on shared
host cache and compile-admission resources. Those attempts produced no source
compiler diagnostics. The nested lint-contract timeouts were later extended;
the complete six-gate run on HEAD `a994236507ae11a3280018143f8b3b70323b4892`
passed. That run reported 763 Python tests passed, 5 skipped, 1 warning, and 66
snapshots passed, including all nine then-current GIL regression cases.
Markdownlint checked 61 files with no errors, and Nixie validated every diagram.

The first requested CodeRabbit review identified twelve follow-ups, which were
addressed and revalidated before publication. A later review requested the
close-on-close case now recorded in `Progress`; its all-gates result supersedes
the earlier interrupted attempts listed here. This section records those
interruptions without attributing host contention to the implementation.

## Context and orientation

`rust_extension/src/handlers/file/` holds the shared file handler:

- `mod.rs` — the `#[pyclass]` `FemtoFileHandler` and its `#[pymethods]` block
  (`py_new`, `py_handle`, `py_flush`, `py_close`);
- `handler_impl.rs` — `FemtoHandlerTrait::handle`, which sends through a
  `crossbeam_channel::Sender` whose behaviour depends on `OverflowPolicy`;
- `worker.rs` — the file worker thread, `FileCommand`, and the flush
  acknowledgement.

`rust_extension/src/handlers/rotating/python.rs` and
`rust_extension/src/handlers/timed_rotating/python.rs` hold the Python wrappers
`PyRotatingFileHandler` / `PyTimedRotatingFileHandler`, which delegate to core
types built on top of `FemtoFileHandler`.

The precedent is `rust_extension/src/http_handler/handler.rs`, where `py_flush`
accepts `py: Python<'_>` and returns `py.detach(|| self.flush())`.

## Plan of work

### Task 1 — shared `FemtoFileHandler` bindings

In `rust_extension/src/handlers/file/mod.rs`:

- `py_handle`: add `py: Python<'_>`; parse the level and build the record
  before the detached section; wrap only the core `handle` call in
  `py.detach(...)`; map the error to `PyRuntimeError` afterwards.
- `py_flush`: add `py: Python<'_>` and return `py.detach(|| self.flush())`.
- `py_close`: add `py: Python<'_>` and run `self.close()` inside `py.detach`.
- Do not add a Loom cfg guard: these methods consume the `Python<'_>` token
  supplied by PyO3 and do not attach to Python from a Loom thread. Keep the
  existing Loom feature lane compiling.

### Task 2 — rotating and timed-rotating wrappers

Same shape in `handlers/rotating/python.rs` and
`handlers/timed_rotating/python.rs` for `handle`, `flush` and `close`. Parsing
and exception construction stay on the GIL side. Audit builders only.

### Task 3 — concurrency regression tests

A new Python test module running scenarios with
`subprocess.run([sys.executable, "-c", script], timeout=...)` so a GIL deadlock
fails the test rather than hanging the suite. Timeout assertions name the
probable deadlock, and child failures include stderr.

### Task 4 — documentation

`docs/users-guide.md` (contract, concurrency overview, backpressure guidance,
rotating sections), `docs/developers-guide.md` (a section near "Loom models"),
and the `femtologging/overflow_policy.py` module docstring.
