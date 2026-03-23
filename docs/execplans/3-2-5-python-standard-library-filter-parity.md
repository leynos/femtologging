# Deliver Python standard library filter parity for logger and root filters

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

After this change, `femtologging` users can attach Python callback filters to
loggers and root loggers, matching the `logging.Filter` protocol from the
Python standard library. A filter may be either a `logging.Filter` subclass
(implementing `filter(record)`) or a plain callable that accepts one record
argument and returns a truthy/falsy value. When a callback filter accepts a
record, any new attributes it sets on the mutable record view are persisted as
enrichment metadata in the Rust-owned `FemtoLogRecord` before the record is
queued for asynchronous dispatch. This enables middleware-driven contextual
logging patterns such as request correlation IDs injected via `contextvars`.

Additionally, `dictConfig` filter entries gain support for the stdlib factory
form (`"()"`) so users can declare Python callback filters in dictionary-based
configuration alongside the existing `level` and `name` declarative forms.
Mixed declarative/factory forms within a single filter entry are rejected with
`ValueError`.

Observable success after implementation:

1. A Python `logging.Filter` subclass that sets `record.correlation_id` from
   a `contextvar` can be attached to a `FemtoLogger` via the builder API or
   `dictConfig`.
2. The enrichment field `correlation_id` appears in the formatted output and
   is accessible to Python handlers receiving the record dict.
3. A plain callable `lambda record: record.level == "INFO"` can be used as a
   filter, suppressing non-INFO records.
4. A `dictConfig` entry using `"()"` to reference a filter factory class
   produces a working Python callback filter.
5. A `dictConfig` entry mixing `"()"` with `level` or `name` is rejected with
   `ValueError`.
6. Concurrent threads using `contextvar`-driven filters produce correct,
   non-interleaved enrichment.
7. All existing level and name filter tests continue to pass unchanged.

## Context and orientation

This plan implements roadmap items 3.2.5, 3.2.5.1, 3.2.5.2, and 3.2.5.3 as
defined in `docs/roadmap.md`. The architectural direction is set by
[ADR 003](../adr-003-python-stdlib-filter-parity.md), which chose Option B:
evaluate Python callback filters on the producer thread and persist accepted
enrichments into Rust-owned record metadata before queueing.

The design constraints for enrichment persistence are documented in both ADR
003 and
[configuration design section 1.1.1](../configuration-design.md#111-filters):

- Enrichment keys must be strings, must not collide with stdlib `LogRecord`
  attributes or femtologging-reserved metadata keys.
- Values may only be `str`, `int`, `float`, `bool`, or `None`. Non-string
  scalars are stringified before persistence.
- Bounded to 64 keys per record, 64 UTF-8 bytes per key, 1,024 UTF-8 bytes
  per value, and 16 KiB total serialized enrichment payload per record.

### Key files and modules

The following files are most relevant to this work:

**Rust filter system:**

- `rust_extension/src/filters/mod.rs` defines the `FemtoFilter` trait
  (`should_log(&self, record: &FemtoLogRecord) -> bool`), the
  `FilterBuilderTrait`, and the `FilterBuilder` enum. A new Python callback
  filter variant must be added here.
- `rust_extension/src/filters/level_filter.rs` and
  `rust_extension/src/filters/name_filter.rs` are the two existing concrete
  filter implementations. They serve as reference patterns.

**Log record and metadata:**

- `rust_extension/src/log_record.rs` defines `FemtoLogRecord` and
  `RecordMetadata`. The `RecordMetadata.key_values` field
  (`BTreeMap<String, String>`) already stores structured key-value pairs.
  Filter-driven enrichment will also be persisted here (or in a dedicated
  enrichment field, depending on design decisions during implementation).

**Logger dispatch (producer path):**

- `rust_extension/src/logger/mod.rs` contains `FemtoLogger`. The
  `passes_all_filters()` method (line ~402) iterates
  `self.filters: Arc<RwLock<Vec<Arc<dyn FemtoFilter>>>>` and rejects on the
  first `false`. Python callback filters must be invoked here, on the producer
  thread, before the record is queued.
- The `py_log()` method (line ~147) creates a `FemtoLogRecord`, then calls
  `self.log_record(record)` which runs level checks, filter checks, formatting,
  and dispatch in sequence.

**Python callable pattern (reference):**

- `rust_extension/src/formatter/python.rs` demonstrates how Python callables
  are safely adapted: `PythonFormatter` wraps a `Py<PyAny>` in
  `Arc<Mutex<...>>`, calls the callable via `Python::attach`, converts the
  record to a dict using `record_to_dict()`, and extracts the string result.
  This pattern should be followed for Python callback filters.

**Configuration:**

- `femtologging/config.py` contains `_build_filter_from_dict()` (line ~228)
  and `_validate_filter_config_keys()` (line ~212) which currently accept only
  `level` or `name` keys. The `"()"` factory form must be added here.
- `rust_extension/src/config/build.rs` resolves filter builders during
  `build_and_init()`.

**Python bindings and stubs:**

- `femtologging/_femtologging_rs.pyi` contains type stubs for
  `LevelFilterBuilder`, `NameFilterBuilder`, and the `RuntimeFilterBuilder`
  type alias.
- `rust_extension/src/python_module.rs` registers PyO3 classes with the
  Python module.

**Existing tests:**

- `tests/features/filters.feature` and `tests/steps/test_filters_steps.py`
  contain BDD scenarios for level/name filters.
- `tests/test_filters.py` contains Python unit tests for filter AND logic,
  reconfiguration, and clearing.
- `tests/test_dict_config.py` contains dictConfig-related tests.

## Constraints

- Execute Python filter callbacks on the producer thread only. Worker threads
  must never hold Python objects or acquire the GIL for filter evaluation, as
  documented in ADR 003 and `docs/multithreading-in-pyo3.md`.
- Preserve existing `LevelFilterBuilder` and `NameFilterBuilder` behaviour
  and performance. Users who do not configure Python callback filters must not
  pay any runtime cost for this feature.
- The `FemtoFilter` trait signature cannot change because it is a public
  Rust API. The trait method `should_log` takes an immutable record reference.
  Python callback filters need a new trait implementation or a wrapper that
  mutates the record through an alternative mechanism.
- Enrichment values must be persisted as Rust-owned data before the record
  is queued. No Python objects may cross the queue boundary.
- Enrichment bounds (64 keys, 64-byte keys, 1,024-byte values, 16 KiB
  total) must be enforced and violations must produce deterministic errors, not
  silent truncation.
- Do not introduce new external Rust or Python dependencies.
- Public Python-facing APIs must receive matching stubs in
  `femtologging/_femtologging_rs.pyi`.
- New Rust tests must use `rstest` fixtures and parameterised cases,
  following `docs/rust-testing-with-rstest-fixtures.md`.
- New public Rust APIs must receive Rustdoc comments following
  `docs/rust-doctest-dry-guide.md`.
- `make check-fmt`, `make typecheck`, `make lint`, `make test`,
  `make markdownlint`, and `make nixie` must all pass before the work is
  considered complete.
- No single code file may exceed 400 lines, as documented in `AGENTS.md`.
- Comments and documentation must use en-GB-oxendict spelling.
- Update `docs/configuration-design.md` and
  `docs/rust-multithreaded-logging-framework-for-python-design.md` with final
  design decisions.
- Mark roadmap items 3.2.5, 3.2.5.1, 3.2.5.2, and 3.2.5.3 as done in
  `docs/roadmap.md` only after all implementation and validation is complete.

## Tolerances (exception triggers)

- Scope: if the implementation requires changes to more than 25 files or
  roughly 1,200 net lines of code and documentation, stop and escalate with a
  reduced slice.
- Interface: if the `FemtoFilter` trait signature must change in a way that
  breaks existing filter implementations, stop and escalate.
- Dependencies: if a new external Rust crate or Python package is needed,
  stop and escalate.
- Iterations: if any quality gate still fails after five focused fix
  attempts, stop and document the blocker.
- Concurrency: if safe filter invocation requires changing handler
  worker-thread contracts, stop and escalate because ADR 003 explicitly places
  filter evaluation on the producer path.
- Feature boundary: if the design starts requiring handler-level filter
  parity (a non-goal of ADR 003), stop and escalate.
- Ambiguity: if the `FemtoFilter` trait cannot accommodate mutable record
  enrichment without breaking existing filter implementations, present options
  with trade-offs before proceeding.

## Risks

- Risk: the current `FemtoFilter::should_log` method takes an immutable
  record reference. Python callback filters need to mutate the record to add
  enrichment fields. This requires a design choice: either change the filter
  evaluation flow to pass a mutable record view separately, or introduce a new
  filter trait/method. Severity: medium. Likelihood: certain (this is a known
  design gap). Mitigation: introduce a `PythonCallbackFilter` that implements
  `FemtoFilter` by returning true/false for the `should_log` call, but also
  exposes a separate enrichment extraction method. The logger's producer-path
  code will call the Python callback with a mutable record view, capture
  enrichment, and then persist it into the record metadata before the
  `should_log` result is evaluated. Alternatively, change the filter evaluation
  loop to pass a mutable record reference and update the two existing filter
  implementations (which do not mutate).

- Risk: producer-path callback execution increases hot-path latency for
  configurations using Python filters, because the GIL must be acquired for
  each callback invocation. Severity: medium. Likelihood: high. Mitigation:
  this is an accepted trade-off per ADR 003. Ensure existing Rust-only filter
  paths remain unchanged and Python callback filters are only invoked when
  explicitly configured. Document the performance implication.

- Risk: Python filter callbacks may raise exceptions during `filter(record)`
  calls. Unhandled exceptions could crash the producer thread. Severity: high.
  Likelihood: medium. Mitigation: catch all Python exceptions during callback
  invocation and treat them as filter rejection (the record is dropped). Log a
  warning via the `log` crate so the failure is observable without crashing.

- Risk: enrichment bound enforcement may reject legitimate callback-produced
  fields, causing silent data loss. Severity: low. Likelihood: low. Mitigation:
  when enrichment bounds are exceeded, log a warning naming the filter and the
  violated bound. The record still passes the filter (it was accepted), but
  excess enrichment fields are not persisted.

- Risk: `dictConfig` factory parsing via `"()"` requires importing and
  instantiating arbitrary Python classes. Malformed class references could
  produce confusing errors. Severity: low. Likelihood: medium. Mitigation:
  validate the factory callable path using the same `ast.literal_eval` and
  import resolution patterns used by stdlib `logging.config.dictConfig`.
  Produce clear `ValueError` messages naming the filter ID and the failed
  import path.

- Risk: concurrent threads using `contextvar`-driven enrichment could
  produce interleaved or incorrect values if the enrichment extraction is not
  thread-isolated. Severity: high. Likelihood: low. Mitigation: enrichment is
  extracted on the producer thread immediately after the Python callback
  returns, before the record is queued. Each producer thread has its own
  `contextvar` state, so there is no cross-thread leakage. Integration tests
  will verify this.

## Plan of work

### Stage A: design the Python callback filter adapter (Rust)

This stage introduces the Rust-side representation of a Python callback filter
without yet wiring it into the logger dispatch or configuration paths.

#### A.1. Create `rust_extension/src/filters/python_callback.rs`

Define a `PythonCallbackFilter` struct that wraps a Python callable in
`Arc<Mutex<Py<PyAny>>>`, following the pattern established by `PythonFormatter`
in `rust_extension/src/formatter/python.rs`.

The struct must implement `FemtoFilter` (and therefore be `Send + Sync`). The
`should_log` method acquires the GIL via `Python::attach`, builds a mutable
record view as a Python dict (reusing `record_to_dict` or a variant), calls the
Python callable, extracts the boolean result, and captures any new attributes
set on the record view as enrichment.

To support enrichment extraction, the record view dict should be compared
before and after the callback call. New or modified keys that pass enrichment
validation (type, size, collision checks) are collected into a
`BTreeMap<String, String>`.

Since `FemtoFilter::should_log` takes `&FemtoLogRecord` (immutable), the
enrichment data cannot be written back through the trait method. Instead, the
`PythonCallbackFilter` should expose a separate method such as
`filter_with_enrichment(&self, record: &FemtoLogRecord) -> FilterResult` where
`FilterResult` is a struct containing the boolean decision and the optional
enrichment map. The logger's producer path will call this method for Python
callback filters and persist enrichment into the record's metadata before
queueing.

The `should_log` trait implementation can delegate to `filter_with_enrichment`
and discard the enrichment, so the filter remains usable through the standard
trait interface (though without enrichment).

Define enrichment validation helpers:

- `validate_enrichment_key(key: &str) -> Result<(), EnrichmentError>`:
  checks that the key is a non-empty string, does not exceed 64 UTF-8 bytes,
  and does not collide with reserved stdlib `LogRecord` attribute names (a
  static set: `name`, `msg`, `args`, `levelname`, `levelno`, `pathname`,
  `filename`, `module`, `exc_info`, `exc_text`, `stack_info`, `lineno`,
  `funcName`, `created`, `msecs`, `relativeCreated`, `thread`, `threadName`,
  `process`, `processName`, `message`, `asctime`, `taskName`) or
  femtologging-reserved keys (`logger`, `level`, `metadata`).
- `validate_enrichment_value(value: &str) -> Result<(), EnrichmentError>`:
  checks that the stringified value does not exceed 1,024 UTF-8 bytes.
- `validate_enrichment_total(map: &BTreeMap<String, String>)`: checks
  that the map has at most 64 keys and total serialised size does not exceed 16
  KiB. Returns `Result<(), EnrichmentError>`.

#### A.2. Add `PythonCallbackFilter` to `FilterBuilder` enum

Add a `PythonCallback(PythonCallbackFilter)` variant to the `FilterBuilder`
enum in `rust_extension/src/filters/mod.rs`. Update the `build()`, `From`, and
Python `FromPyObject` implementations accordingly. The `FromPyObject`
implementation should try the existing `LevelFilterBuilder` and
`NameFilterBuilder` extractions first (preserving current behaviour), then fall
back to attempting extraction as a Python callable or `filter(record)` object.

#### A.3. Add `PythonCallbackFilterBuilder`

Create a builder struct (`PythonCallbackFilterBuilder`) that accepts a Python
callable or `logging.Filter`-like object and produces a `PythonCallbackFilter`.
This builder should be exposed to Python via PyO3 so that users can construct
it explicitly in the builder API:

```python
from femtologging import ConfigBuilder, PythonCallbackFilterBuilder


def my_filter(record):
    record["correlation_id"] = get_correlation_id()
    return True


cb = ConfigBuilder()
cb.with_filter("correlate", PythonCallbackFilterBuilder(my_filter))
```

Register the new class in `rust_extension/src/python_module.rs` and add stubs
to `femtologging/_femtologging_rs.pyi`.

#### A.4. Rust unit tests for enrichment validation

Add `rstest`-parameterised tests in a new test module
`rust_extension/src/filters/python_callback_tests.rs` covering:

- Reserved key rejection (each reserved key in the static set).
- Key length validation (exactly 64 bytes passes, 65 bytes fails).
- Value length validation (exactly 1,024 bytes passes, 1,025 bytes fails).
- Total key count validation (64 keys passes, 65 keys fails).
- Total payload size validation.
- Type validation: only `str`, `int`, `float`, `bool`, and `None` values
  are accepted; other types are rejected.
- Non-string scalar stringification: `int`, `float`, `bool`, and `None`
  are converted to their Python string representations.

Validation:

```plaintext
set -o pipefail && make check-fmt 2>&1 | tee /tmp/3-2-5-a-check-fmt.log
set -o pipefail && make lint 2>&1 | tee /tmp/3-2-5-a-lint.log
PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0 cargo test \
  --manifest-path rust_extension/Cargo.toml \
  --no-default-features --features python \
  -- --test-threads=1 2>&1 | tee /tmp/3-2-5-a-test.log
```

### Stage B: wire Python callback filters into the producer path

This stage modifies the logger's dispatch flow to invoke Python callback
filters and persist enrichment before queueing.

#### B.1. Modify the filter evaluation loop in `FemtoLogger`

In `rust_extension/src/logger/mod.rs`, the `passes_all_filters` method
currently iterates over `Arc<dyn FemtoFilter>` values and calls `should_log`.
For Python callback filters, the logger must instead call
`filter_with_enrichment` and collect the enrichment map.

Introduce a new method such as
`apply_filters(&self, record: &mut FemtoLogRecord) -> bool` that:

1. Iterates over configured filters.
2. For each filter, checks whether it is a `PythonCallbackFilter` (via
   `Any::downcast_ref` on the `Arc<dyn FemtoFilter>` inner value).
3. If it is a Python callback filter, calls `filter_with_enrichment()`.
   On rejection, returns `false` immediately. On acceptance, merges the
   enrichment map into the record's `metadata.key_values`.
4. If it is a Rust-native filter, calls `should_log()` as before.
5. Returns `true` only if all filters accept.

Replace the `passes_all_filters` call site in `log_record()` with
`apply_filters()`, passing a `&mut FemtoLogRecord`.

This approach preserves zero-cost evaluation for Rust-only filter
configurations (the downcast check is a single vtable comparison) and only
acquires the GIL when a Python callback filter is actually configured.

#### B.2. Handle callback errors gracefully

If a Python callback filter raises an exception:

1. Catch the `PyErr`.
2. Log a warning via the `log` crate: `"Python filter callback raised an
   exception; record dropped: {err}"`.
3. Treat the record as rejected (return `false`).

This matches the principle of deterministic error handling stated in ADR 003.

#### B.3. Enrichment persistence

The enrichment map extracted from a Python callback filter is merged into
`record.metadata.key_values`. This field already exists as a
`BTreeMap<String, String>` on `RecordMetadata` and is already serialised into
the Python dict by `record_to_dict()` in
`rust_extension/src/formatter/python.rs`. This means enrichment fields will
automatically be visible to Python handlers and formatters without additional
plumbing.

If enrichment validation fails (bounds exceeded), log a warning and skip the
offending enrichment entries but still accept the record (the filter returned
`true`).

Validation:

```plaintext
set -o pipefail && make check-fmt 2>&1 | tee /tmp/3-2-5-b-check-fmt.log
set -o pipefail && make lint 2>&1 | tee /tmp/3-2-5-b-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/3-2-5-b-test.log
```

### Stage C: extend `dictConfig` with factory filter parsing

This stage implements roadmap item 3.2.5.2 by extending the Python-side
`dictConfig` filter parsing to support the `"()"` factory form.

#### C.1. Update `_validate_filter_config_keys` and `_build_filter_from_dict`

In `femtologging/config.py`:

1. Update `_validate_filter_config_keys` to accept three modes:
   - Factory mode: `"()"` is present. `level` and `name` must be absent.
     Other keys besides `"()"` are passed as keyword arguments to the
     factory.
   - Declarative mode: exactly one of `level` or `name` is present.
     `"()"` must be absent.
   - Mixed/ambiguous: any other combination raises `ValueError`.

2. Update `_build_filter_from_dict` to handle the factory case:
   - Extract the `"()"` value as a dotted import path string.
   - Import the class or callable using a helper similar to stdlib's
     `logging.config` resolution (split on `.`, import the module, get
     the attribute).
   - Instantiate the factory with any remaining keyword arguments from
     the filter entry (excluding `"()"`).
   - Wrap the resulting object in a `PythonCallbackFilterBuilder`.

#### C.2. Add a factory import resolver

Add a helper function `_resolve_factory(dotted_path: str) -> object` in
`femtologging/config.py` (or a new `femtologging/_filter_factory.py` if
`config.py` approaches the 400-line limit) that:

1. Splits the dotted path on `.`.
2. Progressively imports from the leftmost component until the longest
   importable module is found.
3. Resolves the remaining components as attribute lookups.
4. Raises `ValueError` with a clear message if import or attribute
   resolution fails.

This mirrors the resolution logic in `logging.config.dictConfig` for handler
factories.

#### C.3. Python unit tests for factory filter parsing

Add tests in `tests/test_dict_config.py` (or a new focused test file if the
existing file is near the 400-line limit) covering:

- Valid factory form: `{"()": "path.to.MyFilter"}` produces a working
  Python callback filter.
- Factory with keyword arguments: `{"()": "path.to.MyFilter", "arg1":
  "value1"}` passes `arg1` to the factory constructor.
- Mixed form rejection: `{"()": "path.to.MyFilter", "level": "INFO"}`
  raises `ValueError`.
- Mixed form rejection: `{"()": "path.to.MyFilter", "name": "app"}`
  raises `ValueError`.
- Invalid factory path: `{"()": "nonexistent.module.Filter"}` raises
  `ValueError` with a clear message.
- Factory returning non-callable: raises `TypeError`.

Validation:

```plaintext
set -o pipefail && make check-fmt 2>&1 | tee /tmp/3-2-5-c-check-fmt.log
set -o pipefail && make typecheck 2>&1 | tee /tmp/3-2-5-c-typecheck.log
set -o pipefail && make lint 2>&1 | tee /tmp/3-2-5-c-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/3-2-5-c-test.log
```

### Stage D: Python behavioural tests and snapshot coverage

This stage implements the BDD and snapshot test coverage required by roadmap
item 3.2.5.3 and the project's testing conventions.

#### D.1. BDD feature file for Python callback filters

Create `tests/features/python_callback_filters.feature` with scenarios:

- **Happy path: callable filter accepts and enriches a record.** A
  callable that sets `record["request_id"] = "abc-123"` and returns `True` is
  attached. The emitted record contains `request_id` in its metadata.
- **Happy path: `logging.Filter` subclass rejects a record.** A
  `logging.Filter` subclass whose `filter()` returns `False` suppresses the
  record.
- **Happy path: enrichment visible in formatted output.** A callback
  filter adds `correlation_id` and the formatter output includes it.
- **Unhappy path: callback raises exception.** A filter that raises
  `RuntimeError` causes the record to be dropped with a warning.
- **Unhappy path: enrichment key collision with reserved name.** A
  callback sets `record["levelname"] = "CUSTOM"`. The reserved key is rejected
  (not persisted) but the record still passes.
- **Unhappy path: enrichment value exceeds size limit.** A callback
  sets a value exceeding 1,024 bytes. The oversized value is not persisted but
  the record still passes.
- **Edge case: filter returns falsy non-bool (0, empty string, None).**
  The record is rejected.
- **Edge case: filter returns truthy non-bool (1, non-empty string).**
  The record is accepted.
- **Edge case: multiple filters including both Rust-native and Python
  callback.** AND logic applies: all must accept.

#### D.2. BDD step definitions

Create `tests/steps/test_python_callback_filters_steps.py` implementing the
scenarios above. Use the existing `RecordCollector` pattern from
`tests/steps/test_logging_macros_steps.py` for capturing records.

#### D.3. Syrupy snapshot tests

Add snapshot assertions for:

- The `PythonCallbackFilterBuilder.as_dict()` output.
- The enriched record dict captured by a Python handler after a callback
  filter adds enrichment fields.
- The `dictConfig` round-trip shape for a configuration containing a
  factory filter.

#### D.4. Concurrency and contextvar integration tests

Add Python tests (in `tests/test_filters.py` or a new
`tests/test_callback_filter_concurrency.py`) covering:

- Multiple threads each setting a different `contextvar` value and
  logging through the same logger with a callback filter that reads the
  `contextvar`. Assert that each thread's records contain only that thread's
  `contextvar` value.
- An `asyncio` test with multiple coroutines using `contextvars` and
  a callback filter. Assert correct per-task enrichment.

Validation:

```plaintext
set -o pipefail && make check-fmt 2>&1 | tee /tmp/3-2-5-d-check-fmt.log
set -o pipefail && make typecheck 2>&1 | tee /tmp/3-2-5-d-typecheck.log
set -o pipefail && make lint 2>&1 | tee /tmp/3-2-5-d-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/3-2-5-d-test.log
```

### Stage E: documentation and roadmap completion

#### E.1. Update design documents

Update `docs/configuration-design.md` section 1.1.1 (Filters) to record the
shipped Python callback filter support, including the builder API, the
enrichment persistence contract, and the `dictConfig` factory form.

Update `docs/rust-multithreaded-logging-framework-for-python-design.md` to
reflect that Python callback filters are evaluated on the producer path and
enrichment is persisted into Rust-owned metadata before queueing.

#### E.2. Update roadmap

Mark roadmap items 3.2.5, 3.2.5.1, 3.2.5.2, and 3.2.5.3 as done in
`docs/roadmap.md`.

#### E.3. Final validation

Run the full validation suite:

```plaintext
set -o pipefail && make fmt 2>&1 | tee /tmp/3-2-5-e-fmt.log
set -o pipefail && make check-fmt 2>&1 | tee /tmp/3-2-5-e-check-fmt.log
set -o pipefail && make typecheck 2>&1 | tee /tmp/3-2-5-e-typecheck.log
set -o pipefail && make lint 2>&1 | tee /tmp/3-2-5-e-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/3-2-5-e-test.log
set -o pipefail && make markdownlint 2>&1 | tee /tmp/3-2-5-e-markdownlint.log
set -o pipefail && make nixie 2>&1 | tee /tmp/3-2-5-e-nixie.log
```

## Interfaces and dependencies

### New Rust types

In `rust_extension/src/filters/python_callback.rs`:

```rust
/// Result of evaluating a Python callback filter.
pub struct FilterResult {
    /// Whether the record passed the filter.
    pub accepted: bool,
    /// Enrichment fields to merge into the record's metadata.
    /// Empty if the filter rejected the record or produced no enrichment.
    pub enrichment: BTreeMap<String, String>,
}

/// A filter backed by a Python callable or `logging.Filter` object.
///
/// The callable is invoked on the producer thread via `Python::attach`.
/// Enrichment fields set on the mutable record view are extracted and
/// persisted into Rust-owned metadata before the record is queued.
pub struct PythonCallbackFilter {
    callable: Arc<Mutex<Py<PyAny>>>,
    description: String,
}

impl PythonCallbackFilter {
    /// Evaluate the filter and extract enrichment.
    pub fn filter_with_enrichment(
        &self,
        record: &FemtoLogRecord,
    ) -> FilterResult { /* ... */ }
}

impl FemtoFilter for PythonCallbackFilter {
    fn should_log(&self, record: &FemtoLogRecord) -> bool {
        self.filter_with_enrichment(record).accepted
    }
}
```

In `rust_extension/src/filters/enrichment.rs`:

```rust
/// Errors produced during enrichment validation.
#[derive(Debug, Error)]
pub enum EnrichmentError {
    #[error("enrichment key {key:?} collides with reserved attribute")]
    ReservedKey { key: String },
    #[error("enrichment key exceeds 64-byte limit: {len} bytes")]
    KeyTooLong { len: usize },
    #[error("enrichment value exceeds 1024-byte limit: {len} bytes")]
    ValueTooLong { len: usize },
    #[error("enrichment exceeds 64-key limit: {count} keys")]
    TooManyKeys { count: usize },
    #[error("enrichment total size exceeds 16 KiB limit: {size} bytes")]
    TotalTooLarge { size: usize },
    #[error("unsupported enrichment value type: {type_name}")]
    UnsupportedType { type_name: String },
}

/// Validate an enrichment key.
pub fn validate_enrichment_key(key: &str) -> Result<(), EnrichmentError>;

/// Validate a stringified enrichment value.
pub fn validate_enrichment_value(value: &str) -> Result<(), EnrichmentError>;

/// Validate the total enrichment map.
pub fn validate_enrichment_total(
    map: &BTreeMap<String, String>,
) -> Result<(), EnrichmentError>;
```

### New Python types

```python
class PythonCallbackFilterBuilder:
    """Builder for a Python callback filter.

    Accepts a callable that takes a single record argument (a mutable
    dict) and returns a truthy/falsy value, or an object with a
    ``filter(record)`` method.
    """

    def __init__(self, callback: Callable | object) -> None: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> object: ...
```

### Modified Python functions

In `femtologging/config.py`:

- `_validate_filter_config_keys(fid, data)` gains awareness of the
  `"()"` key and enforces mutual exclusion with `level`/`name`.
- `_build_filter_from_dict(fid, data)` gains a factory branch that
  resolves `"()"` and wraps the result in `PythonCallbackFilterBuilder`.

### No new external dependencies

All work uses existing crates (`pyo3`, `thiserror`, `parking_lot`,
`crossbeam-channel`, `log`) and Python packages (`pytest`, `pytest-bdd`,
`syrupy`).

## Validation and acceptance

Quality criteria (what "done" means):

- `make check-fmt` passes (both Rust and Python formatting).
- `make typecheck` passes (Python type checking with `ty`).
- `make lint` passes (ruff + three clippy configurations).
- `make test` passes (all Rust test suites + all pytest tests including
  new BDD scenarios and snapshot tests).
- `make markdownlint` passes on all documentation changes.
- `make nixie` passes on all Mermaid diagrams.
- New `rstest` parameterised cases exist for enrichment validation.
- New `pytest-bdd` scenarios cover happy paths, unhappy paths, and edge
  cases for Python callback filters.
- New syrupy snapshots cover builder output, enriched record dicts, and
  dictConfig round-trips.
- Concurrency tests verify contextvar isolation across threads and
  async tasks.
- Roadmap items 3.2.5, 3.2.5.1, 3.2.5.2, and 3.2.5.3 are marked done.
- Design documents are updated with the shipped behaviour.

Quality method (how to check):

```plaintext
set -o pipefail && make fmt 2>&1 | tee /tmp/3-2-5-final-fmt.log
set -o pipefail && make check-fmt 2>&1 | tee /tmp/3-2-5-final-check-fmt.log
set -o pipefail && make typecheck 2>&1 | tee /tmp/3-2-5-final-typecheck.log
set -o pipefail && make lint 2>&1 | tee /tmp/3-2-5-final-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/3-2-5-final-test.log
set -o pipefail && make markdownlint 2>&1 | tee /tmp/3-2-5-final-markdownlint.log
set -o pipefail && make nixie 2>&1 | tee /tmp/3-2-5-final-nixie.log
```

## Idempotence and recovery

Each stage ends with a validation step. If a stage fails partway through, the
changes from that stage can be reverted without affecting prior stages. The
feature flag gating (`#[cfg(feature = "python")]`) on the new
`PythonCallbackFilter` code means that non-Python Rust builds are unaffected.

`make test` rebuilds the virtualenv and re-installs the package, so re-running
after a partial failure is safe. `make fmt` is idempotent.

## Progress

- [ ] Write ExecPlan draft.
- [ ] Obtain approval.
- [ ] Stage A: design and implement the Python callback filter adapter.
- [ ] Stage B: wire filters into the producer path.
- [ ] Stage C: extend dictConfig with factory filter parsing.
- [ ] Stage D: add BDD, snapshot, and concurrency tests.
- [ ] Stage E: update documentation and close roadmap items.

## Surprises & discoveries

(None yet.)

## Decision log

(None yet.)

## Outcomes & retrospective

(Not yet started.)
