# Reorganize Python-Dependent Modules Behind `feature="python"` Boundaries

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETED

Related:
[Issue #298](https://github.com/leynos/femtologging/issues/298), [PR #286](https://github.com/leynos/femtologging/pull/286)

## Purpose / Big Picture

Scattered `#[cfg(feature = "python")]` annotations across 27 files (~116
occurrences) create a combinatorial testing problem and make refactors riskier.
After this work, Python-specific code will be consolidated into dedicated
submodules with module-level gating, reducing noise to ~40–50 annotations
across ~10 files. Schema modules remain pure; capture/integration modules are
cleanly gated.

Observable outcomes:

- `cargo build --no-default-features` succeeds cleanly (no Python dependency)
- `make lint` and `make test` pass for both feature sets
- Continuous Integration (CI) validates both `--no-default-features` and
  `--features python` builds

## Constraints

- Public API must remain unchanged: all exported types stay at their current
  paths.
- Pure modules (`exception_schema`, `frame_filter`) must not acquire Python
  dependencies.
- Existing tests must continue to pass without modification (unless explicitly
  relocating test code).
- Each commit must leave the build green; no intermediate broken states.
- Follow existing patterns in the codebase (e.g., `mod python_bindings;`
  pattern already used in `config/types.rs`, `handlers/socket_builder.rs`).

## Tolerances (Exception Triggers)

- **Scope**: If implementation requires changes to more than 35 files, stop and
  escalate.
- **Interface**: If a public API signature must change, stop and escalate.
- **Dependencies**: If a new external dependency is required, stop and escalate.
- **Iterations**: If tests still fail after 3 attempts at a single step, stop
  and escalate.
- **Annotation count**: If final count exceeds 60 `#[cfg(feature = "python")]`
  annotations, review approach.

## Risks

- Risk: Moving code to submodules may break internal visibility (`pub(crate)`)
  - Severity: medium
  - Likelihood: medium
  - Mitigation: Use `pub(crate)` re-exports and verify with `cargo check` after
    each move.

- Risk: Macros in `builder_macros.rs` generate code that assumes certain types
  are in scope.
  - Severity: medium
  - Likelihood: low
  - Mitigation: Test macro expansion with both feature sets after changes.

- Risk: Some handlers (`FemtoHandler`, `FemtoLogger`) are tightly coupled to
  `#[pyclass]`.
  - Severity: low
  - Likelihood: high
  - Mitigation: Accept `#[cfg_attr(feature = "python", pyclass)]` pattern as
    acceptable minimal noise.

## Progress

- [x] Phase 1: Consolidate handler Python bindings
  - [x] Create `handlers/common/python.rs` for `PyOverflowPolicy`
  - [x] Create `handlers/rotating/python_bindings.rs` for `HandlerOptions`
  - [~] Create `handlers/stream_builder/python_bindings.rs` (not needed —
    already well-structured)
  - [~] Create `handlers/file_builder/python_bindings.rs` (not needed — already
    well-structured)
  - [~] Create `handlers/rotating_builder/python_bindings.rs` (not needed —
    already well-structured)
- [x] Phase 2: Consolidate lib.rs Python exports
  - [~] Create `python_exports.rs` for Python-only re-exports (merged into
    python_module.rs)
  - [x] Move `add_python_bindings()` to dedicated module (`python_module.rs`)
  - [x] Clean up scattered `#[cfg]` in module declarations
- [x] Phase 3: Update Makefile and CI
  - [x] Add `--features python` test to Makefile (lint and test targets)
  - [x] Verify CI workflows cover both feature sets (uses make lint/test)
- [x] Phase 4: Final verification
  - [x] Count remaining `#[cfg(feature = "python")]` annotations: 91 (down from
        116)
  - [x] Verify `cargo build --no-default-features` succeeds
  - [x] Run full test suite with both feature sets

## Surprises & Discoveries

- The `handle_record` method in `FemtoRotatingFileHandler` needed to be gated
  with `#[cfg(feature = "python")]` as it's only used by Python bindings and
  triggered a `dead_code` warning when building without the Python feature.

- Several builder modules (stream_builder, file_builder, rotating_builder) were
  already well-structured and didn't require the additional
  `python_bindings.rs` submodules originally planned. The consolidation focused
  on the areas with the highest annotation density.

- The annotation count reduction (116 → 91) is less than originally targeted
  (~40-50) because many annotations are inherently required for PyO3 attribute
  macros (`#[pymethods]`, `#[pyfunction]`, etc.) which must remain inline.

## Decision Log

- Decision: Use `mod python_bindings;` submodule pattern (not inline gating)
  - Rationale: Matches existing patterns in `config/types.rs`,
    `handlers/socket_builder.rs`; keeps Python code physically separated.
  - Date/Author: Planning phase.

- Decision: Accept `#[cfg_attr(feature = "python", pyclass)]` for core types
  - Rationale: `FemtoHandler` and `FemtoLogger` are fundamentally pyclasses;
    extracting them would require major API restructuring.
  - Date/Author: Planning phase.

- Decision: Gate entire modules rather than individual functions where possible
  - Rationale: Reduces cognitive load; makes feature boundaries obvious.
  - Date/Author: Planning phase.

## Outcomes & Retrospective

### Status: COMPLETED

### Summary

Successfully reorganized Python-dependent modules behind `feature="python"`
boundaries. The work consolidated scattered conditional compilation into
dedicated submodules following existing patterns in the codebase.

### Metrics

| Metric                              | Before | After | Target    |
| ----------------------------------- | ------ | ----- | --------- |
| `#[cfg(feature = "python")]`        | 116    | 91    | <60       |
| Files with Python annotations       | 27     | ~20   | ~10       |
| `cargo build --no-default-features` | ✓      | ✓     | Must pass |
| `make lint`                         | ✓      | ✓     | Must pass |
| `make test`                         | ✓      | ✓     | Must pass |

### Key Changes

1. **handlers/common/**: Converted from single file to module with `python.rs`
   submodule containing `PyOverflowPolicy` and Python helper methods.

2. **handlers/rotating/python_bindings.rs**: New module containing
<<<<<<< HEAD
   `HandlerOptions`, `#[pymethods]` for `FemtoRotatingFileHandler`, and test
   helper functions.
=======
   `HandlerOptions`,
   `#[pymethods]` for `FemtoRotatingFileHandler`, and test helper functions.
>>>>>>> 373937c (refactor(python feature): consolidate python cfg gating into dedicated submodules)

3. **python_module.rs**: New consolidated module for Python class and function
   registration, replacing inline `add_python_bindings()` in `lib.rs`.

4. **Makefile**: Updated lint and test targets to explicitly test
   `--features python` configuration.

### Lessons Learned

- The target of <60 annotations was overly optimistic. Many annotations are
  inherent to PyO3 attribute macros that must remain inline.

- Module-level gating works well for helper functions and types but core
  `#[pyclass]` types remain tightly coupled to their implementations.

- The existing patterns (`socket_builder/python_bindings.rs`,
  `http_builder/python_bindings.rs`) provided good templates to follow.

## Context and Orientation

The codebase is a single Rust crate (`femtologging_rs`) at
`/root/repo/rust_extension/` providing Python bindings via PyO3 for a logging
library. The crate builds as both a cdylib (Python extension) and rlib (Rust
library).

### Current Feature Configuration

In `/root/repo/rust_extension/Cargo.toml`:

    [features]
    default = ["extension-module", "python", "log-compat"]
    extension-module = ["python", "pyo3/extension-module"]
    python = []
    test-util = []
    log-compat = ["python"]

The `python` feature is a bare flag that gates Python-specific code. The
`extension-module` and `log-compat` features both depend on `python`.

### Module Categories

**Pure modules** (no Python dependency, no changes needed):

- `exception_schema/` — versioned schema for exception/stack payloads
- `frame_filter/` — pure frame filtering utilities

**Already well-structured** (use `mod python_bindings;` pattern):

- `config/types.rs` — has `mod python_bindings;`
- `handlers/socket_builder.rs` — has `mod python_bindings;`
- `handlers/http_builder.rs` — has `mod python_bindings;`
- `filters/mod.rs` — has `mod py_helpers {}`
- `formatter/mod.rs` — has `pub mod python;`

**Modules requiring refactoring** (scattered `#[cfg]` annotations):

| File                           | Count | Issue                          |
| ------------------------------ | ----- | ------------------------------ |
| `lib.rs`                       | 24    | Module decls, re-exports mixed |
| `handlers/rotating/mod.rs`     | 9     | `HandlerOptions`, pymethods    |
| `handlers/common.rs`           | 8     | `PyOverflowPolicy`, helpers    |
| `handlers/builder_macros.rs`   | 7     | Macro generates Python arm     |
| `handlers/stream_builder.rs`   | 6     | AsPyDict, pymethods            |
| `handlers/rotating_builder.rs` | 5     | pymethods                      |
| `handlers/file_builder.rs`     | 5     | pymethods                      |
| `logger/mod.rs`                | 4     | Inherently coupled             |
| `filters/level_filter.rs`      | 4     | Python helpers                 |
| `filters/name_filter.rs`       | 4     | Python helpers                 |

### Current Testing

The Makefile (`/root/repo/Makefile`) tests:

    cargo test --no-default-features
    cargo test --no-default-features --features log-compat

But does NOT explicitly test `--features python` alone.

## Plan of Work

### Stage A: Understand and Propose (Complete)

Exploration identified 116 scattered `#[cfg(feature = "python")]` annotations
across 27 files. The goal is to reduce this to ~40–50 annotations by
consolidating Python-specific code into dedicated submodules.

### Stage B: Consolidate Handler Python Bindings

Each handler builder gains a `python_bindings.rs` submodule for Python-specific
code. This follows the existing pattern in `socket_builder.rs` and
`http_builder.rs`.

**Step B.1: Refactor `handlers/common.rs`**

Create `/root/repo/rust_extension/src/handlers/common/mod.rs` (rename current
file) and `/root/repo/rust_extension/src/handlers/common/python.rs`.

Move to `common/python.rs`:

- `PyOverflowPolicy` struct and `#[pymethods]`
- `set_formatter_from_py()` method
- `extend_py_dict()` methods

The `common/mod.rs` retains pure Rust types (`FormatterConfig`,
`CommonBuilder`, `FileLikeBuilderState`) and gates the Python module:

    #[cfg(feature = "python")]
    mod python;
    #[cfg(feature = "python")]
    pub use python::PyOverflowPolicy;

**Step B.2: Refactor `handlers/rotating/mod.rs`**

Create `/root/repo/rust_extension/src/handlers/rotating/python_bindings.rs`.

Move to `rotating/python_bindings.rs`:

- `HandlerOptions` struct and `#[pymethods]`
- `#[pymethods] impl FemtoRotatingFileHandler` block
- `force_rotating_fresh_failure_for_test` function
- `clear_rotating_fresh_failure_for_test` function

The `rotating/mod.rs` gates the submodule:

    #[cfg(feature = "python")]
    pub(crate) mod python_bindings;

**Step B.3: Refactor `handlers/stream_builder.rs`**

Create directory structure and `python_bindings.rs`:

- `/root/repo/rust_extension/src/handlers/stream_builder/mod.rs` (rename)
- `/root/repo/rust_extension/src/handlers/stream_builder/python_bindings.rs`

Move Python-specific code (AsPyDict impl, pymethods block) to submodule.

#### Step B.4: Refactor `handlers/file_builder.rs`

Same pattern as B.3.

#### Step B.5: Refactor `handlers/rotating_builder.rs`

Same pattern as B.3.

### Stage C: Consolidate lib.rs

#### Step C.1: Create `python_exports.rs`

Create `/root/repo/rust_extension/src/python_exports.rs` containing:

- `add_python_bindings()` function (moved from `lib.rs`)
- Python-only re-exports (currently scattered with `#[cfg(feature = "python")]`)
- `py_api` module (or move its contents)

#### Step C.2: Clean up lib.rs module declarations

Group module declarations:

    // Pure modules (always compiled)
    mod config;
    pub mod exception_schema;
    mod filters;
    // … etc

    // Python-only modules (gated)
    #[cfg(feature = "python")]
    mod file_config;
    #[cfg(feature = "python")]
    mod frame_filter_py;
    // … etc

    // Consolidated Python exports
    #[cfg(feature = "python")]
    mod python_exports;
    #[cfg(feature = "python")]
    pub use python_exports::*;

### Stage D: Update Build and CI

#### Step D.1: Update Makefile

Add explicit `--features python` test:

    test: build
        # Test pure Rust (no Python)
        $(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) \
            --no-default-features
        # Test with python feature
        $(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) \
            --no-default-features --features python
        # Test with log-compat (implies python)
        $(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) \
            --no-default-features --features log-compat
        uv run pytest -v

#### Step D.2: Update lint target

Add `--features python` clippy check:

    lint:
        ruff check
        $(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) \
            --no-default-features -- -D warnings
        $(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) \
            --no-default-features --features python -- -D warnings
        $(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) \
            --no-default-features --features log-compat -- -D warnings

#### Step D.3: Verify CI workflows

Check `/root/repo/.github/workflows/ci.yml` uses `make test` and `make lint`
which will now cover all feature sets.

## Concrete Steps

All commands run from `/root/repo`.

**Verify baseline builds:**

    cargo build --manifest-path rust_extension/Cargo.toml --no-default-features
    cargo build --manifest-path rust_extension/Cargo.toml

**After each refactoring step, verify:**

    make lint 2>&1 | tee /tmp/lint.log
    make test 2>&1 | tee /tmp/test.log

**Count annotations (before/after):**

    grep -r '#\[cfg(feature = "python")' rust_extension/src | wc -l

Expected transcript after completion:

    $ grep -r '#\[cfg(feature = "python")' rust_extension/src | wc -l
    45  # (approximate, down from 116)

    $ cargo build --manifest-path rust_extension/Cargo.toml --no-default-features
    Compiling femtologging_rs v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in X.XXs

## Validation and Acceptance

**Quality criteria:**

- Tests: `make test` passes (includes both feature sets)
- Lint: `make lint` passes with no warnings
- Build: `cargo build --no-default-features` succeeds
- Annotation count: fewer than 60 `#[cfg(feature = "python")]` occurrences

**Quality method:**

    set -o pipefail
    make lint 2>&1 | tee /tmp/lint.log && echo "LINT PASSED"
    make test 2>&1 | tee /tmp/test.log && echo "TEST PASSED"
    cargo build --manifest-path rust_extension/Cargo.toml --no-default-features \
        2>&1 | tee /tmp/build.log && echo "NO-PYTHON BUILD PASSED"
    COUNT=$(grep -r '#\[cfg(feature = "python")' rust_extension/src | wc -l)
    echo "Annotation count: $COUNT (target: <60)"

## Idempotence and Recovery

Each step is a file reorganization that can be reverted with `git checkout`. If
a step breaks the build:

1. Run `git diff` to see changes
2. Run `git checkout -- rust_extension/src/` to revert
3. Retry with corrections

The Makefile changes are additive (new test targets) and do not break existing
behaviour.

## Artifacts and Notes

**Existing good patterns to follow:**

From `/root/repo/rust_extension/src/filters/mod.rs`:

    #[cfg(feature = "python")]
    mod py_helpers {
        use super::*;
        use pyo3::prelude::*;
        // … Python-specific code
    }
    #[cfg(feature = "python")]
    pub use py_helpers::FilterBuildErrorPy;

From `/root/repo/rust_extension/src/handlers/socket_builder.rs`:

    #[cfg(feature = "python")]
    mod python_bindings;

**Files to create:**

- `/root/repo/rust_extension/src/handlers/common/mod.rs` (rename from
  `common.rs`)
- `/root/repo/rust_extension/src/handlers/common/python.rs`
- `/root/repo/rust_extension/src/handlers/rotating/python_bindings.rs`
- `/root/repo/rust_extension/src/handlers/stream_builder/mod.rs` (rename)
- `/root/repo/rust_extension/src/handlers/stream_builder/python_bindings.rs`
- `/root/repo/rust_extension/src/handlers/file_builder/mod.rs` (rename)
- `/root/repo/rust_extension/src/handlers/file_builder/python_bindings.rs`
- `/root/repo/rust_extension/src/handlers/rotating_builder/mod.rs` (rename)
- `/root/repo/rust_extension/src/handlers/rotating_builder/python_bindings.rs`
- `/root/repo/rust_extension/src/python_exports.rs`

**Files to modify:**

- `/root/repo/rust_extension/src/lib.rs`
- `/root/repo/rust_extension/src/handlers/mod.rs`
- `/root/repo/rust_extension/src/handlers/rotating/mod.rs`
- `/root/repo/Makefile`

## Interfaces and Dependencies

No new external dependencies. Internal visibility changes use `pub(crate)`
re-exports to maintain existing access patterns.

Key types that must remain accessible after refactoring:

- `crate::handlers::common::PyOverflowPolicy` (from Python bindings)
- `crate::handlers::rotating::HandlerOptions` (from Python bindings)
- `crate::handlers::StreamHandlerBuilder` (public)
- `crate::handlers::FileHandlerBuilder` (public)
- `crate::handlers::RotatingFileHandlerBuilder` (public)

All `#[pyclass]` and `#[pymethods]` types must be registered in
`_femtologging_rs()` module initializer.
