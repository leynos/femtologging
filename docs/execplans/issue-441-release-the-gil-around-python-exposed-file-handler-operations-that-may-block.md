# Release the GIL around Python-exposed file-handler operations that may block

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IN PROGRESS

Related: [Issue #441](https://github.com/leynos/femtologging/issues/441)

## Purpose / Big Picture

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
- The rotating and timed-rotating wrappers inherit the same contract.
- Python tests, isolated in subprocesses, demonstrate a second thread making
  progress while a direct `handle()` call is blocked.

## Constraints

- Python-visible signatures must not change; the `.pyi` stubs stay as they are.
- Argument parsing and Python exception construction stay on the GIL-owning
  side of the boundary.
- The Rust `flush()`/`close()` implementations, their one-second deadlines, and
  `Drop` are unchanged.
- `--no-default-features` and `--cfg loom` builds must keep compiling; the
  `loom` lane compiles `handlers/file` and `handlers/rotating/core` without the
  Python bindings.
- Loom has no clock, so timed waits are unbounded there. Anything detaching
  must not be compiled into a Loom model.
- `policy="block"` stays available to Python; its shutdown semantics are
  documented rather than changed.

## Tolerances (Exception Triggers)

- If detaching changes observable Python behaviour beyond GIL release, stop and
  re-plan.
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

## Progress

- [x] Reconnaissance: read `handlers/file/mod.rs`, `handler_impl.rs`,
      `rotating/python.rs`, `timed_rotating/python.rs`, the `http_handler`
      precedent, the Loom lane and its model list.
- [x] Branch renamed, Lody session title set.
- [x] Task 1: detach the shared `FemtoFileHandler` bindings (`handle`,
      `flush`, `close`).
- [x] Task 2: detach the rotating and timed-rotating wrappers; audit builders
      (audit found no changes needed).
- [x] Task 3: Python concurrency regression tests
      (`tests/test_file_handler_gil_release.py`).
- [x] Task 4: documentation (user guide, developers guide, `overflow_policy`).
- [x] Build the extension and run the new suite against it: 9/9 pass.
- [x] Confirm the tests fail against the pre-fix bindings: see the negative
      control below. Three tests fail, and only those three.
- [x] Python gates: `make lint-python`, `make check-fmt`, `make fmt`,
      `make markdownlint`, `make nixie` all pass.
- [ ] Run the Rust gates (`make lint-rust`, `make typecheck`, `make test`).
- [ ] Commit in atomic pieces, push, open the draft PR.

## Negative control (evidence that the tests detect the defect)

The suite was run against a deliberately reintroduced defect. Only `py_handle`
in `handlers/file/mod.rs` was changed back to calling
`FemtoHandlerTrait::handle` directly, holding the GIL; `py_flush`, `py_close`,
and both rotating bindings were left fixed. The extension was rebuilt
(`make build`, exit 0) and the suite re-run:

| Test | Result | Child's own diagnosis |
| --- | --- | --- |
| `test_blocking_handle_releases_the_gil[file]` | FAIL | child never finished within the 20s budget |
| `test_timed_handle_releases_the_gil[file]` | FAIL | `recorded only 2 progress record(s) during the 2.006s wait in handle (expected at least 5)` |
| `test_close_while_handle_is_blocked_raises_runtime_error` | FAIL | child never finished within the 20s budget |
| `...[rotating]`, `...[timed-rotating]` | pass | those bindings were left fixed |
| `flush`, `close` | pass | those bindings were left fixed |

The three failures are exactly the three tests that exercise `handle()` on the
file handler. The 2-record result against a 5-record bar, during a 2.006 s
wait where a released wait yields hundreds, is the discriminating signal the
verdict was designed around. The suite is therefore regression evidence rather
than a description of current behaviour.

The fixed source was restored afterwards and the working tree confirmed
identical to the committed revision before rebuilding.

A second, cheaper control was run without any rebuild: the scenario harness was
pointed at `ctypes.PyDLL.usleep` (holds the GIL across the callee, like a
blocked Rust binding) and `ctypes.CDLL.usleep` (releases it). The detector
rejected the first and accepted the second. A pure-bytecode busy-wait would
*not* have been a valid stand-in for the defect, because CPython's eval breaker
drops the GIL every few milliseconds; only a call that blocks below the
interpreter reproduces the freeze.

## Surprises & Discoveries

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

## Decision Log

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

## Outcomes & Retrospective

The implementation is complete and the core claim is now backed by execution
rather than inspection: the fixed build passes 9/9, and a deliberately
reintroduced defect fails exactly the three tests that cover it, with a
discriminating margin (2 progress records against a 5-record bar).

Four of the six gates are green. The Rust gates (`make lint-rust`,
`make typecheck`, `make test`) compile and were blocked for a period by the
compile-admission supervisor reporting `unhealthy`; that is an admission-layer
failure, not a code failure, and is reported rather than worked around.

Two lessons worth carrying forward:

- The GIL verdict has to key on *records inside the call's own window*. An
  ordering assertion ("did another thread run before the call returned?")
  passes for a GIL-holding binding whenever the wait ends on its own deadline,
  so it would have certified the two bounded scenarios as fixed while
  `"block"` remained broken.
- Only a call that blocks below the interpreter models this defect. A
  bytecode-level busy-wait looks equivalent but is not: the eval breaker
  releases the GIL periodically, so the watchdog keeps running and the defect
  vanishes. `ctypes` with a `PyDLL` reproduces it faithfully and costs no
  rebuild.

The first draft of the tests also asserted against module-level constants that
nothing used; the strict Skylos dead-code gate would have rejected those, which
is a useful reminder that the Python gates here catch more than style.

## Context and Orientation

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

The precedent to copy is `rust_extension/src/handlers/mod.rs`… actually
`rust_extension/src/http_handler/handler.rs:199`, where `py_flush` takes
`py: Python<'_>` and returns `py.detach(|| self.flush())`.

## Plan of Work

### Task 1 — shared `FemtoFileHandler` bindings

In `rust_extension/src/handlers/file/mod.rs`:

- `py_handle`: add `py: Python<'_>`; parse the level and build the record
  before the detached section; wrap only the core `handle` call in
  `py.detach(...)`; map the error to `PyRuntimeError` afterwards.
- `py_flush`: add `py: Python<'_>` and return `py.detach(|| self.flush())`.
- `py_close`: add `py: Python<'_>` and run `self.close()` inside `py.detach`.
- Guard the detach with `cfg(not(loom))`, following `logger/worker.rs`.

### Task 2 — rotating and timed-rotating wrappers

Same shape in `handlers/rotating/python.rs` and
`handlers/timed_rotating/python.rs` for `handle`, `flush` and `close`. Parsing
and exception construction stay on the GIL side. Audit builders only.

### Task 3 — concurrency regression tests

A new Python test module running scenarios in a subprocess so a GIL deadlock
fails the test rather than hanging the suite.

### Task 4 — documentation

`docs/users-guide.md` (contract, concurrency overview, backpressure guidance,
rotating sections), `docs/developers-guide.md` (a section near "Loom models"),
and the `femtologging/overflow_policy.py` module docstring.
