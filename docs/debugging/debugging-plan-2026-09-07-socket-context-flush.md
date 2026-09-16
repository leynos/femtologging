# Debugging Plan: Socket context regression test payload propagation

**Generated**: 2026-09-07 **Issue ID**: #417 review follow-up **Severity**: Low
**Falsification sub-agent**: alchemist **Planning agent boundary**: This
document was prepared by the planning agent. Falsification must be executed by
the named sub-agent, not by the planning agent.

## Problem Statement

The integration test registers `FemtoSocketHandler` through
`FemtoLogger.add_handler()`, emits a scoped-context record, and waits for the
server payload with `server.queue.get(timeout=2)`. It then asserts that the
payload contains the scoped key and value. The test must prove record
preservation at the wire boundary without asserting an unrelated flush contract.

## Context Summary

| Aspect              | Details                                                           |
| ------------------- | ----------------------------------------------------------------- |
| First observed      | Full `make test` after the network native-dispatch change         |
| Reproduction rate   | One failure in 510 tests                                          |
| Affected components | `PyHandler`, socket handler, socket serialization path            |
| Recent changes      | Socket handler joined native Python dispatch to preserve metadata |

### Synchronization Evidence

```python
payload = server.queue.get(timeout=2)
assert b"correlation_id" in payload
assert b"abc123" in payload
```

### Information Gaps

- Whether the record reaches the socket server within the two-second timeout.
- Whether native dispatch preserves the scoped fields through socket
  serialization.

______________________________________________________________________

## Hypotheses

### H2: Native `PyHandler` dispatch still drops context before socket serialization

**Claim**: The record is rebuilt or converted before the socket handler
receives it, so the server payload lacks the scoped fields.

**Plausibility**: Medium — this is the behaviour addressed by the review
finding, and the queued payload is the authoritative observation path.

**Prediction**: If the test waits for the payload, it either times out or omits
one of the scoped-context tokens.

#### H2 Falsification Plan

| Step | Action                                              | Expected Negative Result |
| ---- | --------------------------------------------------- | ------------------------ |
| 1    | Inspect the queued payload for both context tokens. | Both tokens are present. |

**Tooling**: The same minimal reproducer and the recording TCP server already
in the test module.

**Confidence on falsification**: Both tokens in the wire payload directly
disprove metadata loss in this path.

______________________________________________________________________

## Recommended Execution Order

1. **H2** — inspect the queued payload to prove or disprove context
   preservation through native dispatch and socket serialization.

## Termination Criteria

- **Root cause identified**: The queued payload proves whether native dispatch
  preserves the scoped fields through socket serialization.
- **Escalation trigger**: No payload arrives within the existing two-second
  socket test timeout.

## Notes for Executing Agent

Do not alter tracked files or run full repository gates. Return a verdict for
each hypothesis and include the exact command and observed values.
