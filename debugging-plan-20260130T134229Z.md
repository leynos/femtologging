# Debugging Plan: Test Suite Hang in Rotating/Log Compat Tests

**Generated**: 2026-01-30T13:42:29Z **Issue ID**:
test-suite-hang-rotating-log-compat **Severity**: High

## Problem Statement

The Rust test suite hangs for ~13 minutes and times out while running a subset
of tests, notably:

- `before_write_reports_rotation_outcome`
  (`handlers::rotating::tests::behaviour`)
- `rotate_falls_back_to_append_when_reopen_fails`
  (`handlers::rotating::tests::behaviour`)
- `adapter_*` (`log_compat::tests`)

Expected behaviour is that all tests complete within the normal `cargo test`
time; instead, these tests remain "running for over 60 seconds" and the make
target is terminated. This blocks CI and local verification.

## Context Summary

| Aspect              | Details                                          |
| ------------------- | ------------------------------------------------ |
| First observed      | Unknown (reported in latest test run)            |
| Reproduction rate   | Unknown; observed in full `make test` run        |
| Affected components | Rotating handler tests, log_compat tests         |
| Recent changes      | Unknown; user reported hang after recent changes |

### Error Artefacts

```text
test handlers::rotating::tests::behaviour::before_write_reports_rotation_outcome
  has been running for over 60 seconds
test handlers::rotating::tests::behaviour::
  rotate_falls_back_to_append_when_reopen_fails has been running for over
  60 seconds
test log_compat::tests::adapter_dispatches_records_to_target_logger
  has been running for over 60 seconds
test log_compat::tests::adapter_normalises_rust_module_targets
  has been running for over 60 seconds
make: *** [Makefile:71: test] Terminated
Error: The operation was canceled.
```

### Information Gaps

- Exact command line and environment settings (e.g., `RUST_TEST_THREADS` or
  `--test-threads`).
- Whether the hang reproduces when running each test individually.
- Recent commits or changes that might affect thread scheduling or global
  logger state.
- Any test logs produced when running with `--nocapture`.

______________________________________________________________________

## Hypotheses

### H1: Parallel test execution causes cross-test interference via global logger

**Claim**: The rotating or log_compat tests share global logger state, and when
executed concurrently the logger initialisation/teardown path deadlocks or
blocks, causing the tests to hang.

**Plausibility**: High — both modules touch global logger state and only some
use `serial_test`; parallel `cargo test` is the default.

**Prediction**: If tests are forced to run single-threaded, the hang disappears.

#### Falsification Plan (H1)

1. Action: Run the full suite with `RUST_TEST_THREADS=1` or
   `-- --test-threads=1`. Expected negative result: Hang still occurs,
   indicating concurrency is not the root cause.
2. Action: Run the two hanging tests individually with
   `-- --nocapture --test-threads=1`. Expected negative result: A single test
   still hangs in isolation.

**Tooling**: `cargo test --manifest-path rust_extension/Cargo.toml <test_name>`
`-- --nocapture --test-threads=1`.

**Confidence on falsification**: Medium-High.

______________________________________________________________________

### H2: Rotating handler tests block on worker-thread shutdown or barrier

**Claim**: The rotating handler tests wait on a barrier or join that is never
released due to a worker-thread stall (e.g., flush/close awaiting ack while the
worker is blocked on I/O or channel operations).

**Plausibility**: Medium — the rotating tests are the first to show prolonged
runtime in the report.

**Prediction**: Running a rotating test in isolation with extra logging reveals
it stalls at a specific barrier/flush/close call.

#### Falsification Plan (H2)

1. Action: Run `before_write_reports_rotation_outcome` with `--nocapture`
   and set `RUST_LOG=trace` if supported by the test logger. Expected negative
   result: Test completes quickly without blocking on flush/close.
2. Action: Temporarily add timeouts or logging around flush/close in the test
   to see where it stops. Expected negative result: No blocking point found;
   test completes even with instrumentation.

**Tooling**: `cargo test --manifest-path rust_extension/Cargo.toml`
`handlers::rotating::tests::behaviour::before_write_reports_rotation_outcome`
`-- --nocapture`.

**Confidence on falsification**: Medium.

______________________________________________________________________

### H3: Log-compat tests hang because a previous test left a worker thread alive

**Claim**: A prior test leaves a logger worker thread running (or holds a
mutex/channel), and the log_compat tests hang while waiting for a flush or
shutdown that never completes.

**Plausibility**: Medium — log_compat tests are reported as hanging alongside
rotating tests, suggesting interference rather than isolated issues.

**Prediction**: Running log_compat tests alone passes; running them after a
specific earlier test reproduces the hang.

#### Falsification Plan (H3)

1. Action: Run
   `log_compat::tests::adapter_dispatches_records_to_target_logger` in
   isolation. Expected negative result: The test still hangs on its own.
2. Action: Re-run with a minimal subset of tests that precede it (binary search
   with `cargo test <subset>`). Expected negative result: The hang appears even
   without preceding tests.

**Tooling**: `cargo test --manifest-path rust_extension/Cargo.toml`
`log_compat::tests::adapter_dispatches_records_to_target_logger -- --nocapture`.

**Confidence on falsification**: Medium.

______________________________________________________________________

### H4: Test environment resource exhaustion (file descriptors or temp files)

**Claim**: The full suite exhausts resources (e.g., file descriptors), causing
later tests that open files or spawn threads to block indefinitely.

**Plausibility**: Low-Medium — the rotating handler uses temp files, and the
hang appears late in the suite.

**Prediction**: Monitoring open file descriptors during the suite shows growth
or limits being hit; reducing suite size avoids the hang.

#### Falsification Plan (H4)

1. Action: Run the suite with `lsof -p <pid>` sampling or `ulimit -n` checks.
   Expected negative result: No abnormal FD growth; limits not reached.
2. Action: Run only the rotating/log_compat tests after a clean start.
   Expected negative result: The hang still occurs despite low resource usage.

**Tooling**: `lsof`, `ulimit -n`, selective `cargo test` runs.

**Confidence on falsification**: Low-Medium.

______________________________________________________________________

## Recommended Execution Order

1. **H1** — quickest to falsify with `--test-threads=1` and isolates
   concurrency.
2. **H3** — targeted isolation of log_compat tests to check cross-test
   effects.
3. **H2** — add logging/timeout instrumentation if isolation still hangs.
4. **H4** — resource checks if above are falsified.

## Termination Criteria

- **Root cause identified**: One hypothesis remains after others are falsified
  and produces a clear fix path (e.g., add `serial_test`, ensure logger
  cleanup, add timeout protections in tests).
- **Escalation trigger**: All hypotheses falsified or hang persists without a
  deterministic reproduction in isolation.

## Notes for Executing Agent

- Use the Makefile targets where possible, but single-test debugging will
  require direct `cargo test` invocations.
- Capture output with `--nocapture` and, if needed, temporary instrumentation
  in the specific test modules.
- Record which exact test order triggers the hang to enable deterministic
  reproduction.
