# Add multi-threading exception capture test

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

Issue: <https://github.com/leynos/femtologging/issues/299> Related PR:
<https://github.com/leynos/femtologging/pull/286>

## Purpose / Big Picture

This change adds a test that validates thread safety of exception capture and
payload handling. After this change, developers can be confident that:

1. Multiple threads can raise and capture exceptions simultaneously without
   cross-thread contamination (each thread's exception payload contains only
   that thread's stack frames and exception data).
2. The exception capture mechanism does not panic under concurrent load.
3. Thread-local exception state remains isolated (stacks and exception messages
   are correct per-thread).

The test will be observable by running `make test` and seeing a new test pass
that spawns N threads, raises exceptions in each, captures payloads, and
asserts payload integrity.

## Constraints

- **No modification to production code**: This is a test-only change. The test
  must exercise existing public APIs without requiring changes to
  `traceback_capture.rs`, `exception_schema/`, or `logger/`.
- **Python test**: The test must be written in Python (not Rust) since the
  exception capture mechanism is exercised through the Python logging
  interface. This matches the existing `test_send_sync.py` pattern.
- **Deterministic execution**: The test must not use `time.sleep()` for
  synchronization. Use barriers, events, or condition variables.
- **Bounded time**: The test must complete within pytest's 30-second timeout.
- **No flaky timing assumptions**: The test must not assume any particular
  ordering of thread execution or timing relationships.

## Tolerances (Exception Triggers)

- **Scope**: If the test requires more than 2 new files or 300 lines of code,
  stop and escalate.
- **Dependencies**: If a new external dependency is required, stop and escalate.
- **Production changes**: If the test reveals a bug requiring production code
  changes, document the finding and escalate before fixing.
- **Iterations**: If tests fail after 3 attempts to fix, stop and escalate.

## Risks

- Risk: Thread scheduling non-determinism could make the test flaky.
  Severity: medium Likelihood: medium Mitigation: Use `threading.Barrier` to
  ensure all threads start together and complete together. Validate payload
  content rather than timing.

- Risk: Python Global Interpreter Lock (GIL) might serialize exception capture,
  masking real concurrency issues. Severity: low Likelihood: high (GIL does
  serialize Python execution) Mitigation: This is acceptable. The GIL ensures
  exception capture is thread-safe at the Python level. The test validates that
  the Rust extension correctly handles the GIL-held capture and that payloads
  don't leak between threads. The existing architecture (capture on caller
  thread with GIL held, worker threads GIL-free) is designed for this.

- Risk: Exception capture might interact with pytest's exception handling.
  Severity: low Likelihood: low Mitigation: Capture exceptions explicitly using
  `sys.exc_info()` within try/except blocks rather than relying on pytest
  fixtures.

## Progress

- [x] (2026-01-18) Create test file
      `tests/test_multithread_exception_capture.py`.
- [x] (2026-01-18) Implement thread worker function that raises a unique
      exception.
- [x] (2026-01-18) Implement barrier-based synchronization for deterministic
      startup.
- [x] (2026-01-18) Implement payload capture and validation.
- [x] (2026-01-18) Add assertions for no cross-thread contamination.
- [x] (2026-01-18) Add assertions for payload integrity (correct exception type,
      message, frames).
- [x] (2026-01-18) Run `make test` and verify the new test passes.
- [x] (2026-01-18) Run `make lint` and `make fmt` to ensure code quality.
- [x] (2026-01-18) Copy ExecPlan to
      `docs/execplans/issue-299-multi-threading-exception-capture-test.md`.

## Surprises & Discoveries

- Observation: Lint rules required exception class to be named with `Error`
  suffix rather than `Exception` suffix. Evidence: ruff N818 error on
  `ThreadSpecificException`. Impact: Renamed to `ThreadSpecificError` to comply
  with project style.

- Observation: Lint rules required abstracting `raise` to inner function.
  Evidence: ruff TRY301 error for raising directly in try block. Impact:
  Created `_raise_thread_error()` helper function.

## Decision Log

- Decision: Write test in Python rather than Rust.
  Rationale: The exception capture mechanism is exercised through Python's
  `exc_info` parameter to logging calls. A Python test exercises the full
  integration path including PyO3 bindings. This matches the existing
  `test_send_sync.py` pattern for concurrency testing. Date/Author: Initial
  plan.

- Decision: Use `threading.Barrier` for synchronization.
  Rationale: Barriers provide deterministic synchronization without sleep-based
  timing. All threads wait at the barrier before proceeding, ensuring they
  start the exception-raising phase together. A second barrier ensures all
  threads complete before validation. Date/Author: Initial plan.

- Decision: Use unique exception messages per thread for identification.
  Rationale: Each thread raises an exception with a message containing its
  thread index (e.g., "Thread 5 exception"). This allows the test to verify
  that each captured payload contains only the correct thread's data, detecting
  any cross-thread contamination. Date/Author: Initial plan.

- Decision: Use custom `ThreadSpecificError` exception class.
  Rationale: A custom exception type makes it easy to verify the captured
  `type_name` field and ensures the test doesn't accidentally catch unrelated
  exceptions. Date/Author: Implementation.

## Outcomes & Retrospective

### What was achieved

- Created `tests/test_multithread_exception_capture.py` with 190 lines of code.
- Test exercises 2, 10, and 50 concurrent threads.
- Test completes in ~0.07 seconds (well under 30-second timeout).
- Test uses deterministic `threading.Barrier` synchronization (no `time.sleep`).
- All 289 tests pass including the new test.
- Lint and format checks pass.

### Lessons learned

- The project's lint rules are strict but promote good exception handling
  patterns (Error suffix, abstract raise, no f-strings in exceptions).
- The `handle_record` structured interface provides clean access to exception
  payloads for testing.
- Barrier-based synchronization is effective for deterministic concurrent tests.

## Context and Orientation

The femtologging library is a high-performance Python logging framework backed
by a Rust extension. Exception capture is handled by the Rust extension in
`rust_extension/src/traceback_capture.rs`.

Key architectural properties:

1. **Exception capture happens on the calling thread** while the Python GIL is
   held (line 5-6 of `traceback_capture.rs`). This ensures thread-local
   exception state (`sys.exc_info()`) is accessed correctly.

2. **Payloads are converted to owned Rust structs** (`ExceptionPayload` in
   `rust_extension/src/exception_schema/mod.rs`). No Python references escape
   the capture phase.

3. **Worker threads are GIL-free**. They receive pre-captured data via
   crossbeam channels.

Relevant existing tests:

- `tests/test_send_sync.py`: Demonstrates the pattern for multithreaded Python
  tests. Uses `pytest.mark.concurrency` and `pytest.mark.send_sync` markers.
  Parametrizes thread counts (1, 10, 100).

- `rust_extension/src/handlers/rotating/tests/concurrency.rs`: Demonstrates
  Rust-side concurrency testing patterns using `AtomicBool`, `Arc<Mutex<>>`,
  and deterministic waiting with timeouts.

- `rust_extension/src/exception_schema/tests/schema_tests.rs`: Contains
  `types_are_send_and_sync()` which asserts that `ExceptionPayload` is
  `Send + Sync`.

Files created:

- `tests/test_multithread_exception_capture.py`: The new test file.
- `docs/execplans/issue-299-multi-threading-exception-capture-test.md`: This
  ExecPlan.

## Plan of Work

### Stage A: Scaffolding (test file structure)

Create `tests/test_multithread_exception_capture.py` with:

1. Module docstring explaining the test purpose.
2. Imports: `threading`, `pytest`, `sys`, and femtologging types.
3. Pytest markers: `pytest.mark.concurrency` and `pytest.mark.send_sync`.
4. A custom exception class `ThreadSpecificError` for clear identification.

Validation: File exists and imports succeed.

### Stage B: Thread worker function

Implement a worker function that:

1. Waits at a `threading.Barrier` for synchronized start.
2. Raises a `ThreadSpecificError` with a unique message containing the
   thread index (e.g., `f"Thread {thread_index} exception"`).
3. Captures the exception using a logging call with `exc_info=True`.
4. Waits at a second barrier for synchronized completion.

The function signature:

    def thread_worker(
        thread_index: int,
        start_barrier: threading.Barrier,
        end_barrier: threading.Barrier,
        logger: FemtoLogger,
    ) -> None:

The handler collects records separately; no results dict needed in the worker.

Validation: Run the test with thread_count=1 to verify single-thread behaviour.

### Stage C: Main test function

Implement `test_multithread_exception_capture` that:

1. Creates a `FemtoLogger` with a `RecordCollectingHandler` (using
   `handle_record` to receive structured payloads).
2. Creates barriers for N+1 parties (N threads + main thread waiting).
3. Spawns N threads (parametrized: 2, 10, 50) each running the worker function.
4. After all threads complete (via barrier), calls `flush_handlers()` on the
   `FemtoLogger` to ensure payloads are flushed to the
   `RecordCollectingHandler` (which receives them via `handle_record`), then
   deletes the logger to ensure worker threads drain completely.
5. Validates:
   - Exactly N records were captured.
   - Each record's `exc_info["message"]` matches a unique thread index.
   - No two records have the same thread index in their message.
   - Each record's `exc_info["frames"]` contains `thread_worker`.
   - All N thread indices are accounted for (no cross-thread contamination).

The test must be parametrized using
`@pytest.mark.parametrize("thread_count", [2, 10, 50])`.

Validation: `pytest tests/test_multithread_exception_capture.py -v` passes.

### Stage D: Final validation and cleanup

1. Run `make test` to ensure all tests pass.
2. Run `make lint` to ensure no linting errors.
3. Run `make fmt` to ensure formatting is correct.
4. Copy this ExecPlan to `docs/execplans/`.

## Concrete Steps

Working directory: `/root/repo`

1. Create the execplans directory if needed:

       mkdir -p docs/execplans

2. Create the test file:

       touch tests/test_multithread_exception_capture.py

3. Write the test implementation (see Plan of Work above).

4. Run the test in isolation:

       uv run pytest tests/test_multithread_exception_capture.py -v

   Expected output: All tests pass, showing parametrized runs for 2, 10, and 50
   threads.

5. Run the full test suite:

       set -o pipefail && make test 2>&1 | tee /tmp/test.log

   Expected output: All tests pass, exit code 0.

6. Run linting:

       set -o pipefail && make lint 2>&1 | tee /tmp/lint.log

   Expected output: No errors, exit code 0.

7. Run formatting check:

       set -o pipefail && make fmt 2>&1 | tee /tmp/fmt.log

   Expected output: No changes needed, exit code 0.

8. Copy ExecPlan to docs:

       cp /root/.claude/plans/wild-churning-spindle.md \
          docs/execplans/issue-299-multi-threading-exception-capture-test.md

## Validation and Acceptance

The test is considered complete when:

1. `make test` passes with the new test included.
2. `make lint` passes with no warnings or errors.
3. `make fmt` produces no changes.
4. The test exercises at least 2, 10, and 50 concurrent threads.
5. The test completes within the pytest 30-second timeout.
6. The test does not use `time.sleep()` for synchronization.
7. The test validates that each thread's exception payload contains only that
   thread's data (exception message, stack frames).

Quality criteria:

- Tests: `make test` passes (all existing tests plus new test).
- Lint/typecheck: `make lint` passes.
- Performance: Test completes in under 5 seconds for all thread counts.

Quality method:

- Run `make test && make lint && make fmt` and verify exit code 0.

## Idempotence and Recovery

The test is fully idempotent. It creates no persistent state. If the test
fails, simply re-run it. The barriers ensure deterministic synchronization
regardless of thread scheduling.

## Artifacts and Notes

Example expected test output:

    tests/test_multithread_exception_capture.py::test_multithread_exception_capture[2] PASSED
    tests/test_multithread_exception_capture.py::test_multithread_exception_capture[10] PASSED
    tests/test_multithread_exception_capture.py::test_multithread_exception_capture[50] PASSED

Example payload validation logic:

    # Extract thread indices from captured records
    captured_indices: set[int] = set()
    for record in handler.records:
        exc_info = record["exc_info"]
        # Parse thread index from message like "Thread 5 exception"
        match = re.search(r"Thread (\d+) exception", exc_info["message"])
        assert match is not None, f"Unexpected message format: {exc_info['message']}"
        thread_idx = int(match.group(1))
        assert thread_idx not in captured_indices, f"Duplicate thread index: {thread_idx}"
        captured_indices.add(thread_idx)

        # Verify stack contains thread_worker
        functions = [f["function"] for f in exc_info["frames"]]
        assert "thread_worker" in functions

    # All thread indices accounted for
    assert captured_indices == set(range(thread_count))

## Interfaces and Dependencies

The test uses existing public APIs:

- `femtologging.FemtoLogger`: The logger class.
- Custom `RecordCollectingHandler` class with `handle_record(self, record)`
  method to receive structured payloads.

Record structure received by `handle_record`:

    {
        "logger": str,
        "level": str,
        "message": str,
        "exc_info": {               # Present when exc_info=True
            "schema_version": int,
            "type_name": str,
            "message": str,         # The exception message
            "frames": [
                {
                    "filename": str,
                    "lineno": int,
                    "function": str,  # Function name for validation
                    …
                },
                …
            ],
            …
        },
        …
    }

The test validates:

- `record["exc_info"]["message"]` contains the thread-specific marker.
- `record["exc_info"]["frames"]` contains a frame with
  `function == "thread_worker"`.
