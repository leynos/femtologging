# ExecPlan: Refactor Exception Formatter (Issues #296 and #297)

| Field          | Value                                                                                                                |
| -------------- | -------------------------------------------------------------------------------------------------------------------- |
| **Status**     | Complete                                                                                                             |
| **Issues**     | [#296](https://github.com/leynos/femtologging/issues/296), [#297](https://github.com/leynos/femtologging/issues/297) |
| **Related PR** | [#286](https://github.com/leynos/femtologging/pull/286)                                                              |
| **Author**     | Terry (AI Agent)                                                                                                     |
| **Created**    | 2026-01-18                                                                                                           |

______________________________________________________________________

## Big Picture

Introduce an `ExceptionFormat` trait that centralizes formatting logic for
exception payloads, ensuring schema evolution requires changes in one place
rather than scattered across formatting code.

## Constraints

- File size limit: 400 lines (per AGENTS.md)
- Behaviour must remain unchanged (snapshot tests must pass)
- Public API must stay stable or changes documented
- Run `make fmt`, `make lint`, `make test` before committing

______________________________________________________________________

## Design

### Trait Definition

```rust
/// Trait for types that can be formatted as human-readable exception output.
///
/// Implementors produce Python-style traceback formatting. This trait
/// centralizes formatting logic so schema evolution requires changes in one
/// place.
pub trait ExceptionFormat {
    /// Format this value into a human-readable string following Python's
    /// traceback formatting style.
    fn format_exception(&self) -> String;
}
```

### Why a Custom Trait (Not `Display`)

1. `Display` is for single-line, user-facing representations
2. Exception formatting is multi-line and complex
3. Custom trait allows future extension (colour, verbosity) without breaking
   changes
4. Follows existing `SchemaVersioned` trait pattern in codebase

### Implementations

```rust
impl ExceptionFormat for StackFrame { ... }
impl ExceptionFormat for StackTracePayload { ... }
impl ExceptionFormat for ExceptionPayload { ... }
```

### Backward Compatibility

Existing public functions delegate to trait methods:

```rust
pub fn format_exception_payload(payload: &ExceptionPayload) -> String {
    payload.format_exception()
}
```

______________________________________________________________________

## File Changes Summary

| File                                        | Action | Description                       |
| ------------------------------------------- | ------ | --------------------------------- |
| `rust_extension/src/formatter/exception.rs` | Edit   | Add trait + impls + tests         |
| `rust_extension/src/formatter/mod.rs`       | Edit   | Add ExceptionFormat to exports    |
| `rust_extension/src/lib.rs`                 | Edit   | Add ExceptionFormat to re-exports |

______________________________________________________________________

## Verification Checklist

- [x] `ExceptionFormat` trait defined with rustdoc
- [x] Trait implemented for `StackFrame`, `StackTracePayload`,
      `ExceptionPayload`
- [x] Existing `format_*` functions delegate to trait methods
- [x] Trait exported from `formatter/mod.rs`
- [x] Trait re-exported from `lib.rs`
- [x] Unit test verifies trait output matches function output
- [x] `make fmt` passes
- [x] `make lint` passes
- [x] `make test` passes (all existing tests including snapshots)
- [x] No file exceeds 400 lines

______________________________________________________________________

## Notes on Issue #296 (Module Organisation)

Issue #296 requested moving exception-formatting helpers to a dedicated module.
Analysis shows:

- `formatter/exception.rs` **already exists** as a dedicated module (295 lines
  after changes)
- This was created in commit `a02f08a` which refactored the monolithic formatter
- The file is well under the 400-line limit
- No further module splitting is necessary

The trait addition (issue #297) addresses the "schema coupling" concern by
providing a single entry point for formatting logic. The module organisation is
already correct.

______________________________________________________________________

## Progress Log

| Date       | Status   | Notes                               |
| ---------- | -------- | ----------------------------------- |
| 2026-01-18 | Draft    | Initial plan created                |
| 2026-01-18 | Complete | Implementation done, all tests pass |
