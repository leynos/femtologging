# Debugging Plan: Socket context regression test flush result

**Generated**: 2026-09-07
**Issue ID**: #417 review follow-up
**Severity**: Low
**Falsification sub-agent**: alchemist
**Planning agent boundary**: This document was prepared by the planning agent.
Falsification must be executed by the named sub-agent, not by the planning
agent.

## Problem Statement

The new integration test registers `FemtoSocketHandler` through
`FemtoLogger.add_handler()`, emits a scoped-context record, and asserts that
`logger.flush_handlers()` succeeds before reading the server payload. The full
test suite consistently reports `False` from `flush_handlers()`. The test must
prove record preservation without asserting an unrelated or invalid flush
contract.

## Context Summary

| Aspect              | Details                                                           |
| ------------------- | ----------------------------------------------------------------- |
| First observed      | Full `make test` after the network native-dispatch change         |
| Reproduction rate   | One failure in 510 tests                                          |
| Affected components | `PyHandler`, socket handler, logger flush path                    |
| Recent changes      | Socket handler joined native Python dispatch to preserve metadata |

### Error Artefacts

```plaintext
AssertionError: logger worker did not flush
assert False
where False = logger.flush_handlers()
```

### Information Gaps

- Whether the record reaches the socket server despite the false flush result.
- Whether the false result originates in the logger barrier or socket-handler
  flushing.

______________________________________________________________________

## Hypotheses

### H1: The logger barrier completes but the socket handler reports a failed flush

**Claim**: The socket worker's flush acknowledgement is not a reliable
completion guarantee at the moment the test invokes it, while native dispatch
still sends the original record.

**Plausibility**: High — the test failure is only the boolean flush assertion,
and the socket handler has its own asynchronous worker.

**Prediction**: Directly awaiting the server queue after emission receives a
payload containing `correlation_id` and `abc123`, even though
`logger.flush_handlers()` returns `False`.

#### H1 Falsification Plan

| Step | Action                                                                                                                              | Expected Negative Result                              |
| ---- | ----------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| 1    | Run only the socket-context test with the flush assertion temporarily bypassed in an untracked copy or a direct minimal reproducer. | No payload arrives, or it lacks either context token. |
| 2    | Report the boolean flush value and payload result without editing tracked files.                                                    | A successful flush disproves the reported condition.  |

**Tooling**: `uv run pytest` minimal reproducer or a temporary untracked script.

**Confidence on falsification**: A received payload decisively shows that the
flush assertion is not needed to prove record preservation.

______________________________________________________________________

### H2: Native `PyHandler` dispatch still drops context before socket serialization

**Claim**: The record is rebuilt or converted before the socket handler
receives it, so the server payload lacks the scoped fields.

**Plausibility**: Medium — this is the behaviour addressed by the review
finding, but the current failure occurs before payload inspection.

**Prediction**: If the test waits for the payload, it either times out or omits
one of the scoped-context tokens.

#### H2 Falsification Plan

| Step | Action                                                            | Expected Negative Result |
| ---- | ----------------------------------------------------------------- | ------------------------ |
| 1    | Inspect the payload from H1's reproducer for both context tokens. | Both tokens are present. |

**Tooling**: The same minimal reproducer and the recording TCP server already
in the test module.

**Confidence on falsification**: Both tokens in the wire payload directly
disprove metadata loss in this path.

______________________________________________________________________

## Recommended Execution Order

1. **H1** — it is the cheapest test and distinguishes the test synchronization
   concern from context propagation.
2. **H2** — the same observation proves or disproves the actual requirement.

## Termination Criteria

- **Root cause identified**: The payload and flush observations distinguish a
  test-only flush assertion from a propagation defect.
- **Escalation trigger**: No payload arrives within the existing two-second
  socket test timeout.

## Notes for Executing Agent

Do not alter tracked files or run full repository gates. Return a verdict for
each hypothesis and include the exact command and observed values.
