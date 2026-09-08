# Debugging Plan: Environment-policy lane timeout after rebase

**Generated**: 2026-09-08
**Issue ID**: Rebase validation for #417
**Severity**: Medium
**Falsification sub-agent**: alchemist
**Planning agent boundary**: This document was prepared by the planning agent.
Falsification must be executed by the named sub-agent, not by the planning
agent.

## Problem Statement

`make test` timed out while `test_a_failing_lane_fails_the_policy_target` ran
the accepting `none` environment-policy lane. The test should complete within
its 30-second timeout after proving that a deliberately invalid lane fails. The
structured-context changes do not intentionally alter environment access, so
the cause must be separated from concurrent Cargo load before a code change is
considered.

## Context Summary

| Aspect              | Details                                                              |
| ------------------- | -------------------------------------------------------------------- |
| First observed      | 2026-09-08, during post-rebase validation                            |
| Reproduction rate   | One full-suite run                                                   |
| Affected components | `tests/test_env_access_policy_contract.py`, Cargo Clippy lane driver |
| Recent changes      | #417 rebased onto `origin/main`, which introduced the policy test    |

### Error Artefacts

```plaintext
FAILED tests/test_env_access_policy_contract.py::test_a_failing_lane_fails_the_policy_target
Failed: Timeout (>30.0s) from pytest-timeout
```

### Information Gaps

- The full-suite run shared the machine with other Cargo processes.

______________________________________________________________________

## Hypotheses

### H1: Concurrent Cargo work exhausted the test's 30-second budget

**Claim**: The accepting `none` lane is correct but was delayed by shared
Cargo-cache or CPU contention during the full suite.

**Plausibility**: High — the test timed out while polling the subprocess, not
after a reported Clippy diagnostic, and the check was first run in a busy
shared environment.

**Prediction**: A direct warm execution of the exact `none` lane finishes with
status zero within 30 seconds once the Cargo cache lock is available.

#### H1 Falsification Plan

| Step | Action                                                                                                                  | Expected negative result                                                            |
| ---- | ----------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| 1    | Run `make lint-env-policy ENV_POLICY_FEATURE_LANES=none ENV_POLICY_LINT_ARGS='-A clippy::all'` and record elapsed time. | A non-zero status or duration above 30 seconds falsifies the contention-only claim. |

**Tooling**: `make`, the repository lane driver, and `time`.

**Confidence on falsification**: High for the accepting lane; it directly
matches the subprocess that the contract test executes after its bad-feature
probe.

______________________________________________________________________

### H2: The #417 Rust changes introduced an environment-policy violation

**Claim**: A changed Rust source or target feature causes the `none` lane to
fail or compile slowly independently of shared system contention.

**Plausibility**: Low — inspection found no environment-access calls in the
Rust diff, but the changed extension remains the compiled subject of the lane.

**Prediction**: The direct `none` lane reports a Clippy or compilation error
that names a source file changed by #417.

#### H2 Falsification Plan

| Step | Action                                                                                     | Expected negative result                                          |
| ---- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------- |
| 1    | Inspect the direct lane output from H1 for a diagnostic naming a changed #417 Rust source. | A successful lane with no source diagnostic falsifies this claim. |

**Tooling**: The output captured for H1 and
`git diff origin/main -- rust_extension`.

**Confidence on falsification**: High for an environment-policy regression in
the `none` lane; other feature lanes remain outside this symptom.

______________________________________________________________________

## Recommended Execution Order

1. **H1** — it exactly reproduces the accepting subprocess with the smallest
   possible Cargo workload.
2. **H2** — it reuses H1's output and distinguishes a branch regression from
   an environmental delay.

## Termination Criteria

- **Root cause identified**: H1 survives with a fast success, or H2 survives
  with a named changed source diagnostic.
- **Escalation trigger**: If the direct lane is slow without a diagnostic,
  revise the plan to investigate Cargo-cache locking and the test timeout.

## Notes for Executing Agent

Run only the single direct lane described above. Do not edit files or run the
full repository gates. Report a verdict for each hypothesis as falsified,
not-falsified, or inconclusive and include the command status and elapsed time.

## Result

The alchemist ran the exact accepting lane on 2026-09-08. It completed with
status zero in 4.81 seconds and reported no Rust diagnostic. H1 is
not-falsified; H2 is falsified. The original full-suite timeout was therefore
consistent with transient shared Cargo contention rather than a #417 policy
regression.
