# ExecPlan: Refactor Flush API for Type Consistency

**Issue:**
[#168 – Inconsistent flush method naming, types, and semantics](https://github.com/leynos/femtologging/issues/168)

**Status:** Completed — **Criticality:** MEDIUM – API consistency; no runtime risk

______________________________________________________________________

## Big picture

Unify the parameter types of `FileHandlerBuilder.with_flush_record_interval`
and `StreamHandlerBuilder.with_flush_timeout_ms` while preserving their
**distinct and intentional semantics** (record-count interval vs time-based
timeout). This is **Option 3** from the issue: "Maintain separate semantics with
consistent typing."

The public Python API accepts `u64` for both methods; internally, Rust stores
and validates using `NonZeroU64` to enforce non-zero at the type level.
Python-side validation in `py_prelude` blocks converts the incoming `u64` to
`NonZeroU64`, raising `ValueError` for zero before calling the Rust APIs.

The semantic difference is documented in `docs/configuration-design.md` lines
282–286 and is by design: file handlers flush after N records written, stream
handlers perform flush operations that block for a timeout period. The issue is
the type inconsistency (`usize` vs `u64`), not the semantic difference.

______________________________________________________________________

## Constraints

1. **Backward compatibility:** The method name `with_flush_record_interval`
   remains unchanged. The Rust API type changed from `usize` to `NonZeroU64`
   (breaking); the Python API accepts `u64` and remains transparent.
2. **Semantic clarity:** Method names already convey their purpose (`_interval`
   vs `_timeout_ms`). Rename is **not** required.
3. **No new abstractions:** The `FlushTrigger` enum (Option 2) is
   over-engineering for a parameter that differs by design.
4. **Type safety:** Rust methods should use `NonZeroU64` internally, with Python
   accepting `u64` and validating via `py_prelude`.

______________________________________________________________________

## Implementation tasks

### Phase 1: Type unification (Rust)

#### Task 1.1: Update `FileLikeBuilderState` storage type

**File:** `rust_extension/src/handlers/common.rs`

- [x] Change `flush_record_interval: Option<usize>` to
      `flush_record_interval: Option<NonZeroU64>` (line 291)
- [x] Update `set_flush_record_interval` to accept `NonZeroU64`
- [x] Update `validate()` to remove the manual zero check – `NonZeroU64`
      enforces this
- [x] Update `handler_config()` to use `NonZeroU64::get()` with clamping to
      `usize::MAX`
- [x] Update `extend_py_dict()` to use `.get()` for dictionary representation

#### Task 1.2: Update `FileHandlerBuilder` method signature

**File:** `rust_extension/src/handlers/file_builder.rs`

- [x] Change `builder_methods!` declaration for `with_flush_record_interval`:
  - `rust_args: (interval: NonZeroU64)` (was `usize`)
  - `py_args: (interval: u64)` (was `usize`)
  - Added `py_prelude` block to convert `u64` to `NonZeroU64` with
    `PyValueError`
    for zero (matching `StreamHandlerBuilder` pattern)
- [x] Updated doc comment to mention `NonZeroU64` type and validation

#### Task 1.3: Update `RotatingFileHandlerBuilder` method signature

**File:** `rust_extension/src/handlers/rotating_builder.rs`

- [x] Applied same changes as Task 1.2 to `with_flush_record_interval` in
      `builder_methods!` macro invocation
- [x] Added `py_prelude` block to validate zero at the Python boundary

#### Task 1.4: Update `HandlerConfig` compatibility

**File:** `rust_extension/src/handlers/file/mod.rs`

- [x] Verified `HandlerConfig.flush_interval` uses `usize` internally
- [x] Added conversion with clamping: large `u64` values clamp to `usize::MAX`

### Phase 2: Python binding validation

#### Task 2.1: Verify Python tests still pass

**File:** `tests/test_handler_builders.py`

- [x] Ran existing tests for:
  - `test_file_builder_negative_flush_record_interval`
  - `test_file_builder_large_flush_record_interval`
  - `test_file_builder_zero_flush_record_interval`
- [x] Error messages remain consistent ("must be greater than zero")

#### Task 2.2: Add cross-builder consistency tests

**File:** `tests/test_handler_builders.py`

- [x] Added `TestFlushApiConsistency` class with tests:
  - `test_file_and_stream_builders_accept_same_max_value` – verifies both
    accept large values (2^63-1)
  - `test_flush_parameter_error_message_format_consistency` – verifies both
    use "must be greater than zero" pattern
  - `test_rotating_builder_inherits_file_builder_flush_type` – verifies
    `RotatingFileHandlerBuilder` uses same `u64` type

### Phase 3: Documentation

#### Task 3.1: Update `configuration-design.md`

**File:** `docs/configuration-design.md`

- [x] Updated line 223 to show `NonZeroU64` in code example
- [x] Updated lines 288–293 to reflect type unification and reference Issue #168

#### Task 3.2: Update Rust doc comments

**Files:** `file_builder.rs`, `rotating_builder.rs`

- [x] Updated doc comments on `with_flush_record_interval` to mention
      `NonZeroU64`
      type and validation behaviour

### Phase 4: Quality gates

- [x] `make fmt` passes
- [x] `make lint` passes (no Clippy warnings)
- [x] `make test` passes (333 Rust tests)
- [x] `uv run pytest tests/test_handler_builders.py` passes
- [x] Full test suite: `uv run pytest` passes (286 Python tests)

______________________________________________________________________

## Files modified

Table: Files modified in this implementation

| File                                              | Change                                                                                            |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `rust_extension/src/handlers/common.rs`           | Changed `flush_record_interval` type to `Option<NonZeroU64>`, updated setter and accessor methods |
| `rust_extension/src/handlers/file_builder.rs`     | Updated `builder_methods!` macro args, added `py_prelude` validation, updated doc comment         |
| `rust_extension/src/handlers/rotating_builder.rs` | Same as above, updated test to use `NonZeroU64`                                                   |
| `tests/test_handler_builders.py`                  | Added `TestFlushApiConsistency` class with three consistency tests                                |
| `docs/configuration-design.md`                    | Updated code example and documentation to reflect type unification                                |

______________________________________________________________________

## Risks and mitigations

Table: Risks and mitigations

| Risk                                                     | Mitigation                                                                          |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Rust callers using `usize` will see compile error        | Low-impact breaking change; migration is trivial (`NonZeroU64::new(n).expect(...)`) |
| Python callers may pass negative values                  | PyO3 `u64` extraction raises `OverflowError` for negatives (existing behaviour)     |
| Large `u64` values may overflow internal `usize` storage | Values clamp to `usize::MAX` in `handler_config()`                                  |

______________________________________________________________________

## Acceptance criteria (from issue)

- [x] Consistent parameter types (prefer `u64` for both) – **addressed in Phase
      1**
- [x] Clear, self-documenting method names – **already clear; no rename needed**
- [x] Documented semantic differences (if intentional) – **addressed in Phase
      3**
- [x] Updated PyO3 bindings for Python compatibility – **addressed in Phase 1**
- [x] Maintain backward compatibility (if possible) – **Python API unchanged**
- [x] Update documentation and examples – **addressed in Phase 3**
- [x] Add tests for unified API – **addressed in Phase 2**
