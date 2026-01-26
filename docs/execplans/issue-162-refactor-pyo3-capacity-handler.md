# Execution plan: refactor PyO3 capacity handler (issue #162)

PyO3 is the Rust crate providing Python bindings for Rust code.

## Big picture

**Issue:** [#162](https://github.com/leynos/femtologging/issues/162) — DRY
(Don't Repeat Yourself) violation in `py_with_capacity` method across handler
builders.

**Goal:** Eliminate duplication in capacity-setting logic across handler
builders whilst maintaining identical PyO3 binding behaviour and all existing
tests.

**Scope:** Internal refactoring only. No public API changes, no behavioural
changes.

**Criticality:** Low — code quality improvement with no user-facing impact.

______________________________________________________________________

## Current state analysis

### What the issue describes (outdated)

The issue describes duplicate `py_with_capacity` method implementations in
`FileHandlerBuilder` and `StreamHandlerBuilder`. This was accurate when the
issue was raised, but **the codebase has since evolved**.

### Actual current state

The `builder_methods!` macro in `builder_macros.rs` now centralizes method
generation for both Rust and Python bindings. Each builder uses a `capacity`
clause within the macro invocation:

Table: Builder capacity clause mapping

| Builder                      | Module             | Capacity Field Path     |
| ---------------------------- | ------------------ | ----------------------- |
| `FileHandlerBuilder`         | `file_builder`     | `state.set_capacity()`  |
| `StreamHandlerBuilder`       | `stream_builder`   | `common.set_capacity()` |
| `RotatingFileHandlerBuilder` | `rotating_builder` | `state.set_capacity()`  |

The remaining duplication is the **macro invocation pattern** (~4 lines each),
differing only in the field path (`state` vs `common`).

### Architecture context

- **`CommonBuilder`** (in `common` module): Base configuration struct with
  `set_capacity()` method
- **`FileLikeBuilderState`** (in `common` module): Wraps `CommonBuilder`,
  delegates `set_capacity()` to `common.set_capacity()`
- **`builder_methods!`** (in `builder_macros` module): Generates Rust
  consuming methods and `#[pymethods]` wrappers from declarative definitions

### Why the original proposal no longer applies

The issue proposes creating a `#[macro_export]` macro named
`py_common_with_capacity`. However:

- The `builder_methods!` macro already generates the Python wrapper
- The `capacity` clause already extracts the setter logic
- A separate macro would duplicate what `builder_methods!` provides

______________________________________________________________________

## Recommended approach: field naming unification

Since `FileLikeBuilderState` delegates to `CommonBuilder`, unify field naming,
so all builders use the same path:

1. Rename `FileHandlerBuilder.state` → `FileHandlerBuilder.common`
2. Rename `RotatingFileHandlerBuilder.state` →
   `RotatingFileHandlerBuilder.common`
3. All capacity clauses then use `common.set_capacity()`

### Pros

- Minimal code change
- No new macros needed
- Field naming becomes consistent across all builders
- Capacity clauses become identical

### Cons

- Requires updating all `state.` references to `common.` in file/rotating
  builders
- Semantic: `FileLikeBuilderState` contains more than "common" fields
  (overflow policy, flush interval)

### Alternative: keep current state

Given the macro infrastructure already handles most of the DRY concern, the
remaining ~4-line duplication per builder may be acceptable. The refactoring is
optional and primarily cosmetic.

______________________________________________________________________

## Constraints

- All existing tests must pass
- PyO3 bindings must maintain identical behaviour (method names, signatures,
  error messages)
- No changes to public Rust API
- Follow AGENTS.md guidelines: small atomic commits, clippy clean, formatted

______________________________________________________________________

## Implementation tasks

### Phase 1: field renaming

- [x] Rename `FileHandlerBuilder.state` to `FileHandlerBuilder.common`
- [x] Update all `self.state.` references in `file_builder.rs`
- [x] Rename `RotatingFileHandlerBuilder.state` to
      `RotatingFileHandlerBuilder.common`
- [x] Update all `self.state.` references in `rotating_builder.rs`
- [x] Update `builder_methods!` capacity clauses to use `common`

### Phase 2: verification

- [x] Run `make test` — all tests pass
- [x] Run `make lint` — no warnings
- [x] Run `make fmt` — formatting clean
- [x] Verify Python bindings: `with_capacity` method works identically

### Phase 3: commit

- [x] Commit with message referencing issue #162

______________________________________________________________________

## Files to modify

Table: Files requiring modification

| File                                              | Changes                                        |
| ------------------------------------------------- | ---------------------------------------------- |
| `rust_extension/src/handlers/file_builder.rs`     | Rename `state` → `common`, update references   |
| `rust_extension/src/handlers/rotating_builder.rs` | Rename `state` → `common`, update references   |

______________________________________________________________________

## Acceptance criteria (from issue #162)

- [x] Both builders use consistent field naming
- [x] All existing tests continue to pass
- [x] PyO3 bindings maintain identical behaviour

______________________________________________________________________

## Progress

- [x] Analyse current codebase state
- [x] Document current architecture
- [x] Write execution plan
- [x] Implement changes
- [x] Run quality gates
- [x] Commit

______________________________________________________________________

## References

- Issue: [#162](https://github.com/leynos/femtologging/issues/162)
- PR context:
  [#146 (comment)](https://github.com/leynos/femtologging/pull/146#discussion_r2274638553)
