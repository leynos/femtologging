# Bound exception payload capture and contain user-defined stringification

Issue: <https://github.com/leynos/femtologging/issues/442> Branch:
`issue-442-bound-exception-payload-capture-and-contain-user-defined-stringification`
Status: **in progress**

## Big picture

Accepted-by-level records currently build an *unbounded* exception/stack
payload **before** filters run, and exception messages/arguments are produced
by calling user-defined `str()`/`repr()` with no output-size budget. A service
that logs attacker-influenced exception values can therefore spend substantial
CPU and memory on the caller thread constructing payloads that may be discarded
by a later filter, and huge representations enter queued records and downstream
formatters.

The defect is an availability/robustness concern. It is **not** an independent
code-execution primitive: custom `__str__`/`__repr__` already run with
application privileges.

The fix must be an explicit bounded/lazy design, not a blind reorder: filters
may legitimately inspect exception data, and moving all capture after filtering
would break that contract.

### The original finding, re-scoped

The original Codex finding ("Disabled logs still capture unbounded `exc_info`
payloads") overstated the current path: `FemtoLogger.py_log()` already returns
early when the logger threshold rejects the level
(`rust_extension/src/logger/convenience_methods.rs`), so level-disabled records
perform **no** capture today. The residual defect is narrower:

1. Unbounded payload construction between the level gate and filter evaluation.
2. Unbounded user `str()`/`repr()` invocation with no output-size budget.

## Constraints

- Files must stay under 400 lines (AGENTS.md).
- Schema version **stays at 1**: adding optional `#[serde(default)]` fields is a
  non-breaking change per the documented policy in
  `rust_extension/src/exception_schema/mod.rs`.
- Never change filter order. Never move a native filter ahead of a callback
  filter.
- Filter evaluation must never suppress an invalid-`exc_info` `TypeError`.
- Preserve ADR-001 behaviour: builtins-module elision, descendant-empty
  `args_repr`/`notes` for chained exceptions, skipped failing `repr()` elements,
  `SyntaxError.msg` message parity, no suggestion computation.
- Public Python APIs only in the walker (no private CPython structs).
- en-GB-oxendict spelling; `typos` gate enforces it.
- Environment access policy (ADR-006): never read/mutate the process environment
  ambiently.

## Acceptance criteria (from the issue)

- Define limits for exception message length, argument count and representation
  length, note count/length, frame count, chain/group depth, group width, and
  aggregate payload bytes.
- Truncate or omit oversized components without propagating user
  `str()`/`repr()` failures through the logging call.
- Emit a bounded marker indicating truncation/omission.
- Decide whether filters receive lazy exception payloads, a bounded pre-capture,
  or a two-phase filtering API; document the contract.
- Avoid full traceback capture before filters that cannot inspect traceback
  fields.
- Add tests for huge strings/bytes, failing and expensive `__repr__`, many
  args/notes, deep chains/groups, and a filter that rejects the record.
- Add a regression test proving a below-threshold record performs no
  exception/stack capture.

## Design decisions taken

- **Two-phase filtering.** `FemtoFilter::inspects_traceback()` (default `true`;
  `false` for `LevelFilter`/`NameFilter`) lets the producer evaluate the
  leading *non-inspecting prefix* before capture. If that prefix accepts, a
  one-shot capture hook runs, then every remaining filter is evaluated in the
  original order. Native filters are never reordered ahead of callback filters.
- **Bounded eager capture for callback filters.** Callback filters receive a
  bounded, eagerly captured payload — the schema stays simple and no lazy
  Python-facing object graph is introduced.
- **Budgeted Rust walker over real exception objects.** Replaces the
  `TracebackException` traversal. Bounded by depth, node count, aggregate
  bytes, identity-cycle detection, and per-field limits.
- **`PayloadTruncation` marker.** Optional, fixed-size, emitted only when a
  field is non-default. Fixed fields: `message_truncated`, `args_omitted`,
  `args_truncated`, `notes_omitted`, `notes_truncated`, `frames_omitted`,
  `cause_omitted`, `context_omitted`, `exceptions_omitted`, `members_truncated`,
  `failed_elements`, `budget_exhausted`.
- **Budget spend order**: type, message, innermost-to-outermost frames, args,
  notes, cause, context, group members.
- **Group limits**: width 15, depth 10.
- **Failure split**: input-resolution failures (invalid `exc_info`) still raise
  `TypeError`; internal traversal failures degrade to a minimal type-only
  payload with no frames and `budget_exhausted` set.
- **Frame ring buffer** keeps the innermost traceback entries and reports
  `frames_omitted`.

## Task breakdown

### Task 1 — Bounded capture core + schema markers

- [x] `rust_extension/src/traceback_capture/limits.rs`:
      `CaptureLimits::default()`
      + `CaptureBudget`. Count retained UTF-8 bytes and nodes; truncate at char
      boundaries with an ASCII truncation suffix *inside* each limit; pass limits
      by reference; support small injected test limits.
- [x] `PayloadTruncation` in the schema; optional `truncation` fields with serde
      default + `skip_serializing_if`; derive `Eq`; no version bump; update
      constructors, struct literals and schema tests.
- [x] Preserve markers through `apply_frame_transform` without marking
      post-capture filtering.
- [x] Replace `TracebackException` traversal with a bounded walker over real
      exception objects; preserve builtins elision, descendant-empty
      `args_repr`/`notes`, `SyntaxError.msg` parity; do not compute suggestions.
- [x] Apply message/arg/note limits *before* retaining data; slice exact
      built-in strings/bytes/bytearrays before the copying/representation
      operations; contain `str()`/`repr()`/attribute failures; use
      `<exception str() failed>` for failed exception stringification; count
      failures and omissions.
- [x] Bound cause/context traversal with depth, node, aggregate-budget and
      identity-cycle checks; treat group children as a generic sequence; enforce
      group width and depth; skip and count failed children.
- [x] Build frames from frame objects, positions and `linecache`; use `None` for
      unavailable positions/source lines/locals; keep innermost traceback entries
      with a fixed-capacity ring buffer; honour explicit tuple tracebacks.
- [x] Bound stack capture through `inspect.currentframe()`/`f_back`, then
      reverse
      retained frames to oldest-first; record omitted frames.
- [x] Remove the old frame-summary converter and its tests only if unused.
- [x] Split input resolution from non-failing traversal; preserve invalid-input
      errors; on internal walker error attach the minimal type-only payload with
      no frames and `budget_exhausted` set.
- [x] Schema round-trip checks.

### Task 2 — Expose capture markers in every payload consumer

Output must be **unchanged when `truncation` is absent**.

- [x] `rust_extension/src/formatter/exception.rs`: one-line markers for non-zero
      truncation fields — `... N earlier frames omitted ...` before frames,
      chain-truncated lines for omitted causes/contexts, `and N more exceptions`
      for groups, and the summary line for omitted args, omitted notes and
      budget exhaustion; apply frame omission markers to stack payloads; do not
      add a formatter recursion-depth guard.
- [x] `exception_payload_to_py` / `stack_payload_to_py` in
      `rust_extension/src/formatter/python.rs`: add a `truncation` dict only for
      `Some` (this is what exposes markers in the callback filter view).
- [x] `rust_extension/src/frame_filter_py.rs`: preserve the optional
      `truncation` key through frame filtering. The payload is copied whole, so
      no parse is needed; the doc comment and pass-through tests pin that.
- [x] `femtologging/adapter.py`: accept the marker and render the same exception
      and stack omission lines.
- [x] `femtologging/_femtologging_rs.pyi`: no change needed; it does not
      declare payload dict shapes.

### Task 3 — Two-phase filtering with deferred bounded capture

- [x] `FemtoFilter::inspects_traceback()` in
      `rust_extension/src/filters/mod.rs`:
      default `true`; `false` in `LevelFilter` and `NameFilter`; keep the default
      for `PythonCallbackFilter` and unknown filters.
- [x] Staged producer entry point accepting a record and a non-failing one-shot
      capture hook: recheck the threshold, snapshot filters, evaluate only the
      leading non-inspecting prefix before capture; if the prefix accepts, run
      the hook; evaluate all remaining filters in order with the existing
      `FilterContext`; then format and dispatch.
- [x] `log_record()` delegates with a no-op hook; Rust logging paths and bridges
      unchanged.
- [x] `convenience_methods.rs`: retain the early threshold return before record
      construction; resolve/validate `exc_info` *before* the staged call; attach
      bounded payloads from the hook.
- [x] `#[cfg(test)]` thread-local capture counter in `traceback_capture.rs`,
      incremented once per exception traversal and once per stack traversal.

### Task 4 — Regression tests + documentation

- [x] `rust_extension/src/traceback_capture_limits_tests.rs`, registered under
      the existing `test` + `python` gating; real exceptions with small injected
      limits; 1,000 args/notes, 200-level cause chain, cycles, nested and wide
      groups, failed child conversion, frame limits, aggregate exhaustion;
      assert bounded field sizes and exact truncation counts; counters bound
      expensive and failing `__repr__` invocations.
- [x] Update existing capture tests without removing descendant-empty and
      skipped-repr assertions.
- [x] Producer tests: zero capture after native-prefix rejection; capture before
      a callback-first native filter; unchanged order; zero captures below
      threshold.
- [x] Text marker tests and frame-filter marker pass-through tests.
- [x] `tests/test_exception_capture_limits.py`: bounded huge strings and bytes;
      contained failed `__str__`; callback truncation counts; deep chains and
      groups; zero stringification after native-filter rejection; bounded
      payloads for rejecting callbacks; zero stringification below threshold;
      counting exception classes; under 30 s; deep chains built iteratively.
- [x] Docs: ADR 001 (bounded walker, limits, optional marker, input/traversal
      failure split, skipped group children, dropped suggestions; state that user
      stringification *call count* is bounded but individual call *time* is not);
      ADR 003 (leading-prefix native filtering, bounded eager callback payloads,
      unchanged order); user guide; frame-filtering design (distinguish capture
      frame limits from later filter `max_depth`).

## Gates

Sequential only, via `scrutineer`, teed to
`/tmp/$ACTION-$(get-project)-$(git branch --show-current).out`:

`make build`, `fmt`, `check-fmt`, `lint`, `lint-rust-clippy`, `lint-env-policy`,
`markdownlint`, `spelling`, `nixie`, `test`, `typecheck`.

Then `coderabbit review --agent` after each major milestone; clear all concerns
before moving on.

## Progress log

- 2026-10-16 — Branch renamed; Lody session renamed; reconnaissance complete;
  `make build` queued behind the shared compile-admission pool.
- 2026-10-16 — ExecPlan created.
- 2026-10-16 — Task 1 largely written: `payload_truncation` schema markers;
  `traceback_capture/{limits,values,frames,walker}.rs` submodules replacing
  `TracebackException` traversal; panic-contained entry points; test-only
  capture counter; `test_utils/capture_limit_helpers.rs` and
  `traceback_capture_limits_tests.rs` added.
- 2026-10-16 — Superseded `traceback_frames.rs` cluster deleted after audit:
  `extract_frames_from_tb_exception` had no caller once `traceback_payload.rs`
  went, and every helper in
  `traceback_test_helpers.rs`/`frame_assertion_helpers.rs` was used only by the
  two frame test modules. `traceback_test_helpers.rs` trimmed to the four
  constructors the capture tests still use.
- 2026-10-16 — Task 1 verification run queued behind compile admission
  (peak 184 crates ahead in the shared pool, sccache still unreachable).
- 2026-10-16 — First green compile of the new capture core: 30 passed, 11
  failed. Diagnosis (all eleven, five distinct causes):
  1. `build_frame` read `f_code` straight off a *traceback* node. A traceback
     describes a frame through `tb_frame`; only a live frame carries `f_code`
     itself. Every traceback frame therefore failed to build, so
     `frames_omitted` swallowed the whole chain. Fixed by resolving both chain
     shapes through one `resolve()` in `frames.rs`.
  2. `capture_stack` called `inspect.currentframe()` (= `sys._getframe(1)`).
     A Rust or C caller contributes no Python frame to count, so the request
     overran the stack and raised `ValueError('call stack is not deep
     enough')`. Measured directly from a Rust test: `sys._getframe(0)` itself
     raises, so a Rust test thread has *zero* Python frames. Fixed by asking
     for depth `0` and treating the absence of a frame as an empty stack.
  3. `str(exception)` on a multi-argument exception runs `repr` on every
     argument (CPython's `BaseException.__str__`), so the message cost an
     unbounded second pass over user code and a raising `__repr__` destroyed
     the whole message. New `message.rs` assembles the multi-argument form
     from the representations already taken; the single-argument form still
     delegates to Python so `__str__` is still honoured.
  4. `members_truncated` was set only from a *retained* member's marker, so a
     group whose members were dropped outright reported nothing.
  5. Two test scripts were `SyntaxError`s, not capture failures: `x = E() from
     e` (`from` is only legal on `raise`) and a module-scope `global`.
- 2026-10-16 — **Blocker.** Compile admission now refuses every crate:
  `build-limits: the supervisor refused: unhealthy`, for the whole dependency
  graph, not just this worktree. `build-limits-status` exits 1 and reports
  `capacity 4, held 3, UNHEALTHY`, with required recovery action *"Restore
  manager connectivity and account for detached sccache requests; tokens remain
  held until the service is verified stopped and empty."* Per the operating
  rules this is to be reported, not worked around: no unsetting
  `RUSTC_WRAPPER`, no direct `rustc`, no restart of the shared supervisor (it
  is another agent's infrastructure, as is the running `sccache`). No gate can
  run until admission recovers, so Task 1 verification is paused here.
- 2026-10-16 — **Hand-verification while admission is down.** Six more
  behaviours were checked by direct `python3 -I` probe rather than by
  inspection, and every one of them corrected a written assertion:
  1. `tb_next` links *inwards* while `f_back` links *outwards*, so a traceback
     node and a live frame are different shapes — `ChainEntry::resolve()` folds
     both.
  2. A C call boundary pushes no Python frame; `sys._getframe(0)` called from C
     still returns the innermost *Python* frame.
  3. `py.run` contributes **two** `<module>` frames, so a six-deep chain
     measures seven entries.
  4. A Rust-side `PyObject_CallNoArgs` into a script function sees only the
     nested `def`s — the script's module frame is gone.
  5. `PyDict::call_method0("name")` raises `AttributeError`; dict-mapped names
     need `get_item(name).call0()`.
  6. `type(x).__str__ is BaseException.__str__` is `True` for `ValueError` and
     plain subclasses but `False` for `OSError`, `KeyError`, `ExceptionGroup`
     and friends, which is what gates the joined-args message.
- 2026-10-16 — **Fixed a marker leak.** `render_args` wrote
  `args_truncated`/`failed_elements` onto *descendant* markers whose payloads
  retain no `args_repr`, because the flags were written where the render
  happened rather than where the field is kept. Outcomes now travel in
  `RenderedArgs` and are reported by `report(&self, marker)` from the root path
  only.
- 2026-10-16 — **Fixed `render_long_string`.** Truncation was inferred from
  the clamp finding the marked literal too long, so a value cut to *exactly*
  the ceiling reported `truncated: false`. Truncation is now a fact about the
  input. The prefix also reserves room for the two quotes and the marker, so
  the assembled literal is not cut a second time and cannot lose its closing
  quote.
- 2026-10-16 — **Extracted `attrs.rs` and `notes.rs`** from `walker.rs`
  (472 → 386 lines) and consolidated a triplicated `as_u32` and a byte-identical
  `child_attr` onto them.
- 2026-10-16 — **Split four over-long files back under the 400-line ceiling.**
  `exception_schema/mod.rs` (529 → 316) gave up its version machinery to
  `version.rs` and its marker to `truncation.rs`; `schema_tests.rs` (422 → 326)
  gave up the marker round-trips to `truncation_tests.rs`;
  `traceback_capture_limits_tests.rs` (429 → 362) gave up the `__repr__`
  containment tests to `traceback_capture_containment_tests.rs`. Verified
  afterwards that **no `.rs` file exceeds 400 lines**, restoring the HEAD state.
- 2026-10-16 — Removed a **dead helper**: `capture()` in the limits tests was
  defined and never called, which the lint gate would have rejected.
- 2026-10-16 — **Intermediate gap found and later fixed.** At this point in the
  implementation, a *single-argument* exception whose argument was a large
  `bytes`/`bytearray` still materialized the full `str()` before the message
  ceiling applied — `str(ValueError(b'\xab' * 100_000))` built 400 003
  characters and then clamped. The bounded-value helpers now render these
  built-ins from a bounded prefix before message construction; see
  `rust_extension/src/traceback_capture/values.rs`.

- 2026-10-16 — **Gate results, first full run.** Targeted capture tests
  **passed** (45 passed, 0 failed) but emitted three compiler warnings for
  unused imports/dead code at `traceback_capture.rs:58,82,87` — the Task 3
  counters, unused until Task 3 wires them. `make check-fmt` **failed** with 9
  formatting diffs across 7 files. `make lint-rust` produced **no verdict**: it
  was cut off at the 30-minute budget while still compiling dependencies, under
  a queue that peaked ~275 crates deep from other agents. Admission itself
  stayed healthy, and `sccache` came back mid-run.
- 2026-10-16 — Fixed the formatting diffs with `make fmt`; `make check-fmt` is
  now green.
- 2026-10-16 — **Rejected a tempting but wrong fix for the dead-code warnings.**
  The first move was to gate the counter on
  `#[cfg(all(test, feature = "python"))]` with a comment claiming the clippy
  lanes build test code for feature combinations *without* `python`. Reading
  `lib.rs` disproved that: `pub(crate) mod traceback_capture` is itself
  `#[cfg(feature = "python")]`, so no non-python lane ever compiles the module
  and the extra condition is a no-op. Acting on the unverified premise would
  have shipped an inert attribute plus a comment asserting a false rationale.
  Reverted. The warnings were already fixed by simply *using* the counters: two
  tests now pin the instrument's semantics, which is what Task 3's "zero
  captures" assertion will be measured with.
- 2026-10-16 — **Second gate run: targeted capture tests green and clean** —
  47 passed, 0 failed, and **zero** compiler warnings anywhere in the log (the
  two extra tests are the counter pins). Also retrieved the stale first-run
  log's lesson: it predated the counter tests, so its three warnings described
  a tree state that no longer existed. Evidence has a timestamp; re-read it
  before acting on it.

- 2026-10-16 — **Admission refused machine-wide, then recovered.** The lint
  gate's second attempt died at 23:22 with
  `build-limits: the supervisor refused: launch-failed` after sitting at
  `waiting for a compile slot (0 ahead, 45s/90s)`, and `make` reported
  `# Rust lint lane failed: none (exit 101)` at the `lint-env-policy` stage.
  This was **not** a clippy finding: a direct `build-limits-rustc … rustc -vV`
  probe reproduced the same refusal, and three consecutive
  `build-limits-status` calls could not reach
  `/run/user/1000/buildlimits/supervisor.sock` (two timeouts, then
  `[Errno 11] Resource temporarily unavailable`). Per the operating rules this
  is reported, never worked around — no unsetting `RUSTC_WRAPPER`/`RUSTDOC`, no
  calling `rustc` directly, no restarting the shared supervisor or `sccache`.
  It recovered on its own within minutes (`capacity 4, held 0, healthy`), so
  the gate is re-run rather than abandoned. **A lint verdict was never produced
  for this tree**; the exit 101 is evidence about the machine, not about the
  diff.
- 2026-10-24 — Resumed from `0856f6c8` on the issue branch. `make fmt`,
  `make check-fmt`, and the focused capture suite passed (58 tests). The first
  `make lint-rust` lane passed after 39m 34s; its extension-module lane is
  queued behind the shared Cargo package-cache lock. `build-limits-status`
  briefly timed out, then returned healthy with all four slots occupied by
  shared work. No admission recovery action was taken.
- 2026-10-24 — Added Python adapter truncation rendering and regression tests,
  plus the bounded-capture and filter-order contract to ADRs 001/003, the user
  guide, and the frame-filtering design. Python and Markdown gates remain
  pending. No gate overlaps the active `make lint-rust` run.
- 2026-10-24 — Corrected the historical single-argument bytes gap: the current
  `traceback_capture/values.rs` renders bounded `bytes` and `bytearray`
  prefixes before building exception messages; the former “known gap” note
  below describes an earlier intermediate state, not the current candidate.
- 2026-10-26 — Resumed the gate run after an interruption. `make lint-rust`
  passed formatting checks, 11 lint-lane contract tests, every feature lane,
  Rust test-support docs (4/4), and the private-docs lint contract (1/1), then
  failed Whitaker because `traceback_capture/walker.rs` was 410 lines. Moved
  exception and exception-group type checks into `traceback_capture/types.rs`;
  `walker.rs` is now 388 lines. The remaining deterministic gates have not run.
  Resume the full sequential gate list after this fix.
- 2026-10-26 — After the size fix, `make fmt`, `make check-fmt` (188 files),
  and `make build` passed. `make lint` then found Ruff issues in the adapter
  marker helper and its tests: import style, docstring sections, and summary
  complexity. The helpers and test docstrings have been revised; formatting and
  gates are pending again.
- 2026-10-26 — Ruff then reported only NumPy docstring section order in the
  adapter helper. Moved `Yields` and `Returns` before `Examples`; the
  sequential gate suite will be restarted after formatting.
- 2026-10-26 — Interrogate reported 97.5% production docstring coverage but
  did not identify the missing item. Added parameter/return documentation and
  examples to each new payload-formatting helper; formatter and gate reruns
  remain pending.
- 2026-10-26 — Ruff accepted those docs except for `Returns` sections on two
  helpers annotated `-> None`. Removed the sections; `make fmt` and the gate
  rerun are pending.
- 2026-10-26 — Ruff, Interrogate (100%), and standard Pylint passed; the custom
  lint stage required explicit messages on ten assertions in the adapter marker
  tests. Added specific failure explanations; formatting and gates are pending.
- 2026-10-26 — Whitaker found the limits-test module at 412 lines. Moved its
  failing-`__str__` containment test into the neighbouring stringification
  containment module; the limits module is now 394 lines. Formatting and gates
  are pending again.
- 2026-10-26 — `make fmt`, `make check-fmt`, `make build`, `make lint`,
  `make lint-rust`, `make lint-rust-clippy`, and `make lint-env-policy` all
  passed. `make markdownlint` reached spelling and found one Oxford `-ize`
  correction in this plan; corrected it. Remaining spelling, diagram, test, and
  typecheck gates are pending.
- 2026-10-31 — Task 3 and the Rust half of Task 2 completed: the producer now
  evaluates the leading non-inspecting filter prefix, runs the one-shot capture
  hook, then evaluates the remaining filters; `formatter/exception_markers.rs`
  renders bounded omission markers for stacks, frames, chains, groups and the
  summary line. Focused suites green: 58 `traceback_capture` tests and 7
  `producer` tests before this entry.
- 2026-10-31 — Task 4 tests added. `logger/deferred_capture_tests.rs` (ordering,
  prefix rejection, post-capture rejection, threshold, native-filter
  capability) and `logger/deferred_capture_py_tests.rs` (real-traversal
  counter: 0 below threshold, 0 after a native rejection, 2 for an accepted
  exception + stack, and invalid `exc_info` still raising `TypeError` past a
  rejecting filter) — 11 passing. `formatter/exception_marker_tests.rs` — 8
  passing; marker position, cause/context honesty, suppressed context, group
  and summary text, and marker-free output.
  `frame_filter_truncation_tests.rs` — 3 passing; the whole-dict copy keeps
  markers at root, cause and group-member levels.
  `tests/test_exception_capture_limits.py` — 12 passing in 0.99 s; huge
  strings/bytes, contained failing `repr`, 1,000 args/notes caps, 200-link
  cause chains, wide and deep groups, zero stringification below threshold and
  after native-filter rejection, and a rejecting callback observing a bounded,
  marked payload.
- 2026-10-31 — Split the ordering tests from the Python counter tests after the
  combined module reached 436 lines; each is now under the 400-line ceiling.
  `cargo fmt` reformatted the new Rust tests and `ruff format` the new Python
  test; `ruff check` is clean. Full sequential gate suite is next.
- 2026-10-31 — The first full `make lint` run surfaced two dylint findings, both
  fixed without relaxing a gate. `no_expect_outside_tests` rejected
  `exc_info.expect(...)` in `py_log`; `conditional-max-n-branches` rejected the
  three-branch capture guard in `attach_captured_payloads`. Rather than rewrite
  either inline, capture eligibility and validation moved into a new
  `validated_exc_info` helper that returns the validated reference, so `py_log`
  holds `Option<&Bound<PyAny>>` and the hook no longer re-tests eligibility — a
  shape that cannot drift out of step with validation.
- 2026-10-31 — Full gate run: `fmt`, `check-fmt`, `build`, `lint`,
  `lint-rust-clippy`, `lint-env-policy`, `markdownlint`, `spelling`, `nixie` and
  `typecheck` all passed. `make test` was cut off by the harness's 2-hour
  background cap while compiling the `--features python` doctests; the
  `--no-default-features` variant had already passed in full (28 doctests, 12
  ignored) and no suite reported a failure. The remaining variants and
  `pytest --doctest-modules` are being re-run separately.
- 2026-10-31 — Two documentation defects found while reviewing the diff were
  fixed: the new "Bounded exception and stack capture" section had been
  inserted inside the "Emitting records" bullet list, splitting it, and the ADR
  001 locals paragraph had a duplicated clause. `make spelling` also regenerated
  `typos.toml` as a side effect; that unrelated generated change was reverted
  so the commit stays atomic.

## Lessons / notes

- Compile admission is shared machine-wide; a queued build is not a hang. Do not
  kill, retry or duplicate builds.
- `sccache` is currently unreachable (`build-limits-status` reports
  `compiling directly`); builds run uncached — not a failure.
- The worktree shares the git stash stack with other sessions; never use bare
  `git stash`/`git stash pop`.
- **A Rust test thread has no Python frames at all.** `sys._getframe(0)` raises
  `ValueError('call stack is not deep enough')` from a `#[test]`, so anything
  that walks Python frames must be driven from Python to observe a stack.
  `traceback.extract_stack()` is not exempt: called from Rust it happily
  returns one frame — *its own* `extract_stack` frame — which is how the old
  summary-based `capture_stack` passed a non-emptiness assertion while
  capturing nothing of the caller. Prefer `_getframe(0)` and treat its absence
  as "no stack", and pin that with a test that asserts the empty result.
- CPython's `BaseException.__str__` joins `args` with `repr` once there is more
  than one argument. Reading a message with `str(exception)` is therefore a
  *second* unbounded pass over every argument's `__repr__`, and one raising
  `__repr__` destroys the entire message. Build the multi-argument message from
  representations already taken instead.
- 2026-11-02 — Triaged the three `pytest --doctest-modules` failures from the
  full run. Two were caused by this change and are fixed:
  `tests/data/expected_wheel_snapshot.json` now lists the new
  `femtologging/_payload_formatting.py`, and live-stack capture clears
  `colno`/`end_colno` after building frames. The bounded walker had started
  populating column positions from `co_positions()` for stacks as well as
  tracebacks, which made the formatter emit caret underlines that
  `traceback.format_stack()` never produces; `extract_stack()` leaves those
  fields unset, so clearing them restores standard-library parity and the
  recorded `stack_info` snapshot. The third failure,
  `test_a_failing_lane_fails_the_policy_target`, timed out at 620 s while the
  compile pool was saturated and is environmental; it is being re-run.
- 2026-11-17 — `make lint` failed on fifteen `bumpy_road_function` findings,
  nine of them in files this change never touches. The rolling Whitaker tip was
  not at fault: `origin/main` was thirty-three commits ahead and already
  contained `Extract helpers to clear bumpy_road_function findings (#622)`, so
  the branch was rebased onto `c3bd1637` instead of silencing the lint. Nine
  files conflicted. Two needed design work rather than text merging: upstream
  had extracted the Python record schema into `_adapter_record.py`, so the
  truncation marker and formatting moved there behind the new
  `_payload_formatting` module; and upstream's new
  `handler_filters::apply_filter_chain` replaced our duplicate filter loop, so
  the deferred split now calls `apply_filter_segment` twice with one shared
  `FilterContext`. All four Rust test lanes then passed from the rebased tree.
- 2026-11-18 — Two gate failures came from the rebase itself rather than from
  the feature. A nested `reject` callback lost its docstring during conflict
  resolution, which `interrogate` rejected, and the
  `missing_docs_in_private_items` lane caught `should_propagate_to_parent`
  after the upstream doc comment was dropped. Both are restored.
- 2026-11-18 — Applied the nine CodeRabbit findings from the review pass:
  nested payloads now render in `exc_text` (including a member's own markers),
  a displayed cause or context no longer also emits its omission marker, the
  context walker skips the implicit duplicate that `raise ... from e` creates,
  `resolve` type-checks tracebacks and frames before following them, a failed
  argument `__repr__` is no longer double-counted as an omitted argument, the
  type leaf is chosen before clamping, cut literals keep their closing quote,
  and the `bytearray` borrow no longer needs `unsafe`. The rendering moved to a
  new `femtologging/_exception_rendering.py` because the additions pushed
  `_payload_formatting.py` past Pylint's 400-line ceiling; the wheel snapshot
  and the developer guide were updated with it. Two Rust modules then crossed
  Whitaker's `module_max_lines` ceiling: the `Recursion` position moved to
  `traceback_capture/recursion.rs`, and the exception-group limit tests moved to
  `traceback_capture_group_tests.rs`.
