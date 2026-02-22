# Add logging macros and Python convenience functions with source location capture

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

## Purpose / Big Picture

The femtologging roadmap (Phase 1) requires "`debug!`, `info!`, `warn!`, and
`error!` macros that capture source location." After this work, two audiences
gain new capabilities:

1. **Python users** can call `femtologging.info("hello")` (and `debug`,
   `warn`, `error`) without manually retrieving a logger. These module-level
   functions use the root logger by default (matching Python's
   `logging.info()` ergonomics) and accept an optional `name` keyword argument
   to target a named logger. Each call automatically captures the Python
   caller's filename, line number, and function name into the log record's
   metadata.

2. **Rust callers** (within the extension crate or downstream) can use
   `femtolog_info!("logger.name", "message")` (and `_debug`, `_warn`,
   `_error`) macros that capture `file!()`, `line!()`, and `module_path!()`
   at compile time and embed them in the `RecordMetadata`.

Observable success: after implementation, running `uv run python -c "import
femtologging; femtologging.info('hello')"` produces formatted output through
the root logger, and `make test` passes with new Rust unit tests and Python
BDD/snapshot tests exercising both APIs.

## Constraints

- Public API of existing types (`FemtoLogger`, `FemtoLogRecord`,
  `RecordMetadata`, etc.) must remain unchanged.
- `make check-fmt`, `make typecheck`, `make lint`, and `make test` must all
  pass.
- No new external Cargo dependencies.
- Rust macros must not conflict with `log` crate macros (used internally via
  the `log-compat` feature).
- Python convenience functions are gated on `#[cfg(feature = "python")]` to
  match the existing pattern.
- The Rust macros module is always compiled (no feature gate) so pure-Rust
  callers can use it.
- Source files must not exceed 400 lines.
- en-GB-oxendict spelling in comments and documentation.

## Tolerances (Exception Triggers)

- **Scope**: if implementation requires changes to more than 15 files (net),
  stop and escalate.
- **Interface**: if a public API signature must change, stop and escalate.
- **Dependencies**: if a new external dependency is required, stop and
  escalate.
- **Iterations**: if tests still fail after 5 attempts at a single step, stop
  and escalate.

## Risks

- Risk: `sys._getframe(1)` may not be available in all Python implementations
  (e.g., GraalPy, PyPy).
  - Severity: low
  - Likelihood: low
  - Mitigation: Fall back to empty strings / zero line number when frame
    inspection fails. Document this limitation.

- Risk: Widening `dispatch_record` / `is_enabled_for` from `log-compat`-only
  to unconditional may surface dead-code warnings under `--no-default-features`.
  - Severity: low
  - Likelihood: medium
  - Mitigation: Use `expect(dead_code)` if methods are only called from
    feature-gated modules, or restructure to use `log_record` which is already
    unconditional.

- Risk: Frame depth for `sys._getframe` may differ when called through
  different code paths (direct call vs. decorator).
  - Severity: medium
  - Likelihood: low
  - Mitigation: Use depth 0 from the Rust `#[pyfunction]` (the function
    itself is the immediate caller of `_getframe`; depth 0 gives the Rust
    function's frame, depth 1 gives the Python caller). Validate with tests.

## Progress

- [x] Write ExecPlan document.
- [x] Add `pub fn log_with_metadata()` to `FemtoLogger` (instead of widening
  `dispatch_record`).
- [x] Create `rust_extension/src/logging_macros.rs` with Rust macros.
- [x] Create `rust_extension/src/convenience_functions.rs` with Python-callable
  functions.
- [x] Register new functions in `python_module.rs` and `lib.rs`.
- [x] Export convenience functions in `femtologging/__init__.py`.
- [x] Add type stubs to `femtologging/_femtologging_rs.pyi`.
- [x] Write Rust unit tests for macros (6 tests).
- [x] Write Rust unit tests for convenience functions (7 tests).
- [x] Write Python BDD feature file and step definitions (7 scenarios).
- [x] Write snapshot tests for formatted output (1 syrupy snapshot).
- [x] Update `docs/roadmap.md` to mark the macros item as done.
- [x] Run all quality gates and fix any issues.

## Surprises & Discoveries

- `log_with_metadata` needed to be `pub` rather than `pub(crate)` because
  `#[macro_export]` macros are accessible by downstream crates, and
  `pub(crate)` would cause dead-code warnings under `--no-default-features`.
- The `DefaultFormatter` does not include source location metadata in its
  output (`logger [LEVEL] message` only). Source location capture is verified
  in Rust unit tests via `CollectingHandler` rather than in the Python BDD
  tests, where only the formatted string is observable.
- `cargo test --features python` runs extremely slowly (60+ seconds per test)
  due to GIL contention when many tests use `Python::attach`. This is a
  pre-existing issue, not introduced by this change. Running targeted tests
  (e.g., `-- convenience_functions::tests`) completes in milliseconds.
- The frame depth for `sys._getframe` works as 1 (not 2) because Rust
  functions are transparent in the Python frame stack. The Python frame stack
  only sees `[Python caller] -> [pyfunction]`, so depth 1 from the
  `#[pyfunction]` gives the Python caller's frame.

## Decision Log

- Decision: Use prefixed Rust macros (`femtolog_debug!`, etc.) rather than
  bare `debug!`, `info!`, etc.
  - Rationale: The `log` crate's `debug!`, `info!`, `warn!`, `error!` macros
    are already used throughout the codebase (e.g., `log::warn!` in
    `logger/mod.rs`). Using the same names would create ambiguity and require
    qualifying every `log::` call. The `femtolog_` prefix makes provenance
    clear and avoids breakage.
  - Date/Author: Planning phase.

- Decision: Python convenience functions default to the root logger.
  - Rationale: Matches Python's `logging.debug()` / `logging.info()` etc.
    which use the root logger. Users who need a specific logger can pass
    `name="my.logger"`. This is the least-surprise API for Python developers.
  - Date/Author: Planning phase.

- Decision: Implement only the four levels specified in the roadmap (debug,
  info, warn, error), not trace or critical.
  - Rationale: The roadmap explicitly lists these four. Adding trace and
    critical can be done as a follow-up without breaking changes.
  - Date/Author: Planning phase.

- Decision: Use `FemtoLogger.log_record()` (private, unconditional) rather
  than widening `dispatch_record` / `is_enabled_for`.
  - Rationale: `log_record` already performs level checking, filter checking,
    formatting, and dispatch — exactly what we need. It is `fn` (private) but
    we can add a `pub(crate)` method `log_with_metadata` that creates a record
    from metadata and delegates to `log_record`. This avoids widening the
    feature gate on `dispatch_record`, which would risk dead-code warnings.
  - Date/Author: Planning phase.

## Outcomes & Retrospective

All quality gates pass:

- `make check-fmt` — clean (ruff format + cargo fmt)
- `make typecheck` — clean (only a pre-existing `redundant-cast` warning in
  `config_socket_opts.py`)
- `make lint` — clean (ruff check + clippy × 3 feature combos)
- `make test` — Rust: 225 passed (no-default-features), 349 passed (python),
  359 passed (log-compat); Python: 308 passed, 48 snapshots passed

Files created (7):

- `docs/execplans/1-1-3-rust-macros.md`
- `rust_extension/src/logging_macros.rs` (macros + 6 tests)
- `rust_extension/src/convenience_functions.rs` (4 pyfunctions)
- `rust_extension/src/convenience_functions_tests.rs` (7 tests)
- `tests/features/logging_macros.feature` (7 BDD scenarios)
- `tests/steps/test_logging_macros_steps.py` (step definitions)
- `tests/steps/__snapshots__/test_logging_macros_steps.ambr` (syrupy snapshot)

Files modified (5):

- `rust_extension/src/logger/mod.rs` — added `log_with_metadata()`
- `rust_extension/src/lib.rs` — added module declarations
- `rust_extension/src/python_module.rs` — registered 4 pyfunctions
- `femtologging/__init__.py` — exported `debug`, `info`, `warn`, `error`
- `femtologging/_femtologging_rs.pyi` — added type stubs
- `docs/roadmap.md` — marked macros item as done

## Context and Orientation

The femtologging project is a high-performance Python logging library
implemented in Rust via PyO3. The Rust extension crate lives at
`rust_extension/` and is compiled into a shared library that Python imports as
`femtologging._femtologging_rs`.

Key files relevant to this work:

- `rust_extension/src/lib.rs` — Crate root; declares modules, registers the
  PyO3 module, and re-exports public types.
- `rust_extension/src/logger/mod.rs` — `FemtoLogger` struct with `py_log()`,
  `log()`, and the private `log_record()` method that performs level checking,
  filtering, formatting, and dispatch.
- `rust_extension/src/log_record.rs` — `FemtoLogRecord` and `RecordMetadata`
  structs. `FemtoLogRecord::with_metadata()` accepts explicit source location.
- `rust_extension/src/level.rs` — `FemtoLevel` enum (`Trace`, `Debug`, `Info`,
  `Warn`, `Error`, `Critical`).
- `rust_extension/src/manager.rs` — Global logger registry;
  `get_logger(py, name)` returns `Py<FemtoLogger>`.
- `rust_extension/src/python_module.rs` — Consolidates `#[pyfunction]`
  registrations; `register_python_functions()` is called during module init.
- `rust_extension/src/log_compat.rs` — Existing `log` crate bridge; shows the
  pattern for constructing `RecordMetadata` with source location and
  dispatching records.
- `femtologging/__init__.py` — Python package; imports from
  `._femtologging_rs` and re-exports the public API.
- `tests/features/` — Gherkin feature files for BDD tests.
- `tests/steps/` — Python step definitions for `pytest-bdd`.
- `tests/conftest.py` — Shared fixtures including `_clean_logging_manager`.
- `docs/roadmap.md` — Phase 1 item to mark as done.

Build and test commands:

    make build          # Build and install into venv
    make check-fmt      # Verify formatting
    make typecheck      # Static type analysis (ty check)
    make lint           # Clippy + ruff (3 feature combos)
    make test           # Full test suite (3 Rust feature combos + pytest)

## Plan of Work

### Stage A: Scaffolding — Rust Macros Module

Create `rust_extension/src/logging_macros.rs` containing four macros:
`femtolog_debug!`, `femtolog_info!`, `femtolog_warn!`, `femtolog_error!`.

Each macro accepts a logger reference and a message string. It captures
`file!()`, `line!()`, `module_path!()` at the call site, constructs a
`RecordMetadata`, and calls a new `pub(crate)` method
`FemtoLogger::log_with_metadata(level, message, metadata)` which delegates to
the existing private `log_record()`.

The new method on `FemtoLogger`:

    pub(crate) fn log_with_metadata(
        &self,
        level: FemtoLevel,
        message: &str,
        metadata: RecordMetadata,
    ) -> Option<String> {
        let record = FemtoLogRecord::with_metadata(&self.name, level, message, metadata);
        self.log_record(record)
    }

Register the module in `lib.rs`:

    mod logging_macros;

Validation: `cargo test --manifest-path rust_extension/Cargo.toml
--no-default-features` compiles and existing tests pass.

### Stage B: Scaffolding — Python Convenience Functions

Create `rust_extension/src/convenience_functions.rs` (gated on
`#[cfg(feature = "python")]`) containing:

- A shared helper `log_at_level(py, level, message, name)` that:
  1. Resolves the logger via `manager::get_logger(py, name)`.
  2. Calls `sys._getframe(1)` to get the Python caller's frame.
  3. Extracts `f_code.co_filename`, `f_lineno`, `f_code.co_name`.
  4. Builds `RecordMetadata` with the extracted source location.
  5. Calls `logger.borrow(py).log_with_metadata(level, message, metadata)`.
  6. Returns `Option<String>`.

- Four `#[pyfunction]`s (`py_debug`, `py_info`, `py_warn`, `py_error`) that
  delegate to `log_at_level` with the appropriate `FemtoLevel`.

Python signatures:

    def debug(message, /, *, name=None) -> str | None: ...
    def info(message, /, *, name=None) -> str | None: ...
    def warn(message, /, *, name=None) -> str | None: ...
    def error(message, /, *, name=None) -> str | None: ...

Register in `lib.rs`:

    #[cfg(feature = "python")]
    mod convenience_functions;

Register all four in `python_module.rs::register_python_functions()`.

Export from `femtologging/__init__.py` as `debug`, `info`, `warn`, `error`.

Validation: `make build` succeeds and
`uv run python -c "import femtologging; print(femtologging.info('hello'))"`
produces output.

### Stage C: Rust Unit Tests

Add `#[cfg(test)]` blocks or separate test files for:

**Macro tests** (in `logging_macros.rs`):

- Each macro produces a record at the correct level.
- Source location (`file!()`, `line!()`, `module_path!()`) is captured.
- Below-threshold records return `None`.

**Convenience function tests** (in `convenience_functions_tests.rs`):

- Default logger is "root" when `name` is `None`.
- Named logger is used when `name` is provided.
- Level filtering works (below threshold returns `None`).
- Source location from Python frame is captured in metadata.

Use `rstest` fixtures following existing patterns in `log_compat.rs`:
`CollectingHandler`, `unique_logger_name` fixture, `Python::attach`.

Validation: `make test` passes with new tests visible in output.

### Stage D: Python BDD and Snapshot Tests

Create `tests/features/logging_macros.feature` with scenarios:

- `info()` logs a message via root logger.
- `debug()` is suppressed when root level is INFO.
- `error()` with explicit `name` uses a named logger.
- Output of `info()` matches snapshot.
- Source location is captured (file and line present).
- `warn()` at WARN level is emitted.

Create `tests/steps/test_logging_macros_steps.py` with step definitions.

Validation: `uv run pytest tests/steps/test_logging_macros_steps.py -v`
passes.

### Stage E: Documentation and Roadmap

Mark the roadmap checkbox in `docs/roadmap.md`:

    - [x] Provide `debug!`, `info!`, `warn!`, and `error!` macros that capture
      source location.

Update the ExecPlan progress and outcomes sections.

### Stage F: Quality Gates

Run all quality gates:

    set -o pipefail
    make check-fmt 2>&1 | tee /tmp/check-fmt.log
    make typecheck 2>&1 | tee /tmp/typecheck.log
    make lint 2>&1 | tee /tmp/lint.log
    make test 2>&1 | tee /tmp/test.log

Fix any failures. Repeat until all pass.

## Concrete Steps

All commands run from `/home/user/project`.

**After each stage, verify:**

    set -o pipefail
    make lint 2>&1 | tee /tmp/lint.log
    make test 2>&1 | tee /tmp/test.log

## Validation and Acceptance

Quality criteria:

- Tests: `make test` passes (all three Rust feature combos and pytest).
- Lint: `make lint` passes with no warnings.
- Format: `make check-fmt` passes.
- Typecheck: `make typecheck` passes.
- New Rust tests exercise macro source location capture.
- New Python BDD tests verify convenience function behaviour and snapshot
  output.
- Roadmap entry is marked as done.

Quality method:

    set -o pipefail
    make check-fmt 2>&1 | tee /tmp/check-fmt.log && echo "FMT PASSED"
    make typecheck 2>&1 | tee /tmp/typecheck.log && echo "TYPECHECK PASSED"
    make lint 2>&1 | tee /tmp/lint.log && echo "LINT PASSED"
    make test 2>&1 | tee /tmp/test.log && echo "TEST PASSED"

## Idempotence and Recovery

Each stage produces file additions or edits that can be reverted with
`git checkout`. No destructive or irreversible operations are involved.

## Artifacts and Notes

**Existing pattern for source location capture** (from `log_compat.rs`):

    let metadata = RecordMetadata {
        module_path: record.module_path().unwrap_or_default().to_string(),
        filename: record.file().unwrap_or_default().to_string(),
        line_number: record.line().unwrap_or(0),
        ..Default::default()
    };
    let femto_record = FemtoLogRecord::with_metadata(
        logger_name.as_str(), level, &message, metadata,
    );

**Python frame inspection** (planned approach):

    let sys = py.import("sys")?;
    let frame = sys.call_method1("_getframe", (1_i32,))?;
    let code = frame.getattr("f_code")?;
    let filename: String = code.getattr("co_filename")?.extract()?;
    let lineno: u32 = frame.getattr("f_lineno")?.extract()?;
    let funcname: String = code.getattr("co_name")?.extract()?;

## Interfaces and Dependencies

No new external dependencies. Internal interfaces:

**New method on `FemtoLogger`** (in `rust_extension/src/logger/mod.rs`):

    pub(crate) fn log_with_metadata(
        &self,
        level: FemtoLevel,
        message: &str,
        metadata: RecordMetadata,
    ) -> Option<String>

**New Rust macros** (in `rust_extension/src/logging_macros.rs`):

    femtolog_debug!(logger, "message")
    femtolog_info!(logger, "message")
    femtolog_warn!(logger, "message")
    femtolog_error!(logger, "message")

**New Python functions** (in `rust_extension/src/convenience_functions.rs`):

    #[pyfunction]
    fn debug(py, message, /, *, name=None) -> PyResult<Option<String>>

    #[pyfunction]
    fn info(py, message, /, *, name=None) -> PyResult<Option<String>>

    #[pyfunction]
    fn warn(py, message, /, *, name=None) -> PyResult<Option<String>>

    #[pyfunction]
    fn error(py, message, /, *, name=None) -> PyResult<Option<String>>
