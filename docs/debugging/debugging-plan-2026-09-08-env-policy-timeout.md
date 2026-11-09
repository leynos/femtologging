# Debugging Plan: Environment-policy lane timeout after rebase

**Generated**: 2026-09-08 **Issue ID**: Rebase validation for #417
**Severity**: Medium **Falsification sub-agent**: alchemist **Planning agent
boundary**: This document was prepared by the planning agent. Falsification
must be executed by the named sub-agent, not by the planning agent.

**Status**: Reopened on 2026-11-09 by
[#537](https://github.com/leynos/femtologging/issues/537). The 2026-09-08
`Result` below is retained verbatim as history; the revision at the end
supersedes it.

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

- **Consistent with a surviving hypothesis**: H1 survives with a fast success,
  or H2 survives with a named changed source diagnostic.
- **Escalation trigger**: If the direct lane is slow without a diagnostic,
  revise the plan to investigate Cargo-cache locking and the test timeout.

The 2026-09-08 execution treated a fast H1 success as a root cause identified.
That was stronger than the evidence: one fast run is consistent with an
uncontended cache, but cannot exclude a contended stall. See the 2026-11-09
revision.

## Notes for Executing Agent

Run only the single direct lane described above. Do not edit files or run the
full repository gates. Report a verdict for each hypothesis as falsified,
not-falsified, or inconclusive and include the command status and elapsed time.

## Result

> **Superseded on 2026-11-09 by
> [#537](https://github.com/leynos/femtologging/issues/537).** This result
> records one uncontended run. It cannot speak to the 420-second
> package-cache-lock stall recorded in the revision below, and it is retained
> verbatim only as history.

The alchemist ran the exact accepting lane on 2026-09-08. It completed with
status zero in 4.81 seconds and reported no Rust diagnostic. H1 is
not-falsified; H2 is falsified. The original full-suite timeout was therefore
consistent with transient shared Cargo contention rather than a #417 policy
regression.

______________________________________________________________________

## Revision: 2026-10-16 package-cache-lock stall

### Observation

On 2026-10-16 the same accepting `none` lane ran on a host where other agents
were compiling. The command was:

```sh
timeout 420 make lint-env-policy ENV_POLICY_FEATURE_LANES=none \
  ENV_POLICY_LINT_ARGS='-A clippy::all'
echo "exit=$?"
```

It did not finish inside 420 seconds (`exit=124`). Its entire output was:

```plaintext
# Rust lint lane: none
    Blocking waiting for file lock on package cache
make: *** [Makefile:178: lint-env-policy] Error 143
exit=124
```

`Error 143` is the caller's `timeout` sending SIGTERM to `make`; the lane
itself was still blocked. No Clippy diagnostic appeared, so the lane never
reached compilation: it was waiting, not building.

Conditions recorded around the stall:

| Measurement         | Value                                                                     |
| ------------------- | ------------------------------------------------------------------------- |
| Load average, 1 min | 19.25 during the stall; 12.45 when the issue was written                  |
| Admission pool      | `held 4` during the run; a compiler waiting 111.4 s                       |
| Post-stall status   | `capacity 4, held 2, healthy, sccache unreachable, compiling directly`    |
| Queued work         | `waiting 99.3s: rustc in .../cap-primitives-4.0.3`                        |
| File descriptors    | `open descriptors 1687, soft limit 524288, hard limit 524288`             |
| Isolated module run | `tests/test_env_access_policy_contract.py` passed 6 of 6 in 74.42 seconds |

For contrast, a warm, uncontended run of the same lane takes seconds; the
2026-09-08 measurement was 4.81 seconds.

### Why the 2026-09-08 result does not cover this

H1 asked only whether the lane finishes "within 30 seconds once the Cargo cache
lock is available". A single fast run therefore could not distinguish an
uncontended cache from a contended one that happened to be lucky. It recorded
one success and no observation of the lock being held, so it is not evidence
that the lane is independent of that lock.

The bound also changed after the plan was written. The plan's Error Artefacts
quote the then-current global `timeout = 30` from `[tool.pytest.ini_options]`;
the test's own marker has since moved:

| Date       | Bound                                                     | Source                     |
| ---------- | --------------------------------------------------------- | -------------------------- |
| 2026-09-08 | global 30 s                                               | plan's Error Artefacts     |
| 2026-09-15 | 120 s marker                                              | commit `c805961`           |
| 2026-10-15 | 620 s marker                                              | commit `692ef3c9`          |
| 2026-11-04 | 3600 s per subprocess and 7220 s marker (`2 × 3600 + 20`) | commit `0dccd48d`          |
| 2026-11-09 | 120 s per subprocess and 180 s marker                     | this revision (fake Cargo) |

A 4.81-second result cannot speak to a lane whose allowance had risen by two
orders of magnitude. Nor did H1 reproduce the contract's exact argument vector:
it omitted `ENV_POLICY_CARGO_ARGS=--lib`, which the test passed at that time.

### H3: The accepting lane waits on Cargo's package-cache lock

**Claim**: The lane blocks on Cargo's exclusive package-cache lock
(`~/.cargo/.package-cache`) while a co-scheduled Cargo resolve holds it.
`build-limits` admission bounds how many compilers run at once; it does not
bound how long a resolve holds that lock, and with `sccache unreachable` every
cache-touching step is slower.

**Falsification criterion**: "the accepting lane blocks on Cargo's
package-cache lock while a co-scheduled resolve holds it". H3 survives if the
lane prints `Blocking waiting for file lock on package cache` and makes no
progress until the competing job releases the lock. It is falsified if the lane
stalls with no lock message, or if its elapsed time is independent of a
competing resolve.

**Prediction**: with a separate Cargo job holding the package cache, the lane
spends measurable time in `Blocking waiting for file lock on package cache`
before its first Clippy diagnostic; with the cache free, the same lane reaches
Clippy without that message.

Green CI does not retire H3. CI runners are not co-scheduled with other agents'
compilers, so a green `build-test` leg is consistent with a stall that only
appears on a shared host. All four `build-test` legs passed on PR
[#531](https://github.com/leynos/femtologging/pull/531) while the stall was
reachable locally.

### Revised measurement plan

This plan is deliberately wider than the original single-lane scope. Execute it
only when a competing resolve can be held in place; otherwise record "not
collected" rather than inferring a result.

1. Start a separate Cargo job that holds the package cache, for example
   `cargo fetch --locked --manifest-path rust_extension/Cargo.toml` or a build
   in another checkout. Confirm that it holds `~/.cargo/.package-cache` before
   starting the lane.
2. Run the accepting lane without an enclosing `timeout`:

   ```sh
   make lint-env-policy ENV_POLICY_FEATURE_LANES=none \
     ENV_POLICY_LINT_ARGS='-A clippy::all'
   ```

3. Repeat with `--lib --offline` against a warmed cache:

   ```sh
   make lint-env-policy ENV_POLICY_FEATURE_LANES=none \
     ENV_POLICY_CARGO_ARGS='--lib --offline' ENV_POLICY_LINT_ARGS='-A clippy::all'
   ```

4. Repeat each measurement several times. For every run record the lock-wait
   duration until the blocking message clears, total elapsed time, exit status,
   load average and `build-limits-status`.
5. If contention is reproducible, record each run's distribution and
   conditions, then give H3 a verdict. If it is not reproducible, record "not
   collected" with the reason. Do not infer unmeasured results.

**Status: not collected.** The 2026-10-16 run established that a stall can
happen, but it paired no stalled run with an uncontended control and no
repeated series was captured. Holding the cache on demand was not reproduced
for this revision, so no distribution is claimed.

### Decision: remove Cargo from the contract test

The contract that timed out was testing the policy target's status propagation,
not Cargo's scheduling, so the class fix is to stop it needing the
package-cache lock at all. `tests/test_env_access_policy_contract.py` now puts
a recording fake `cargo` first on `PATH` (defined in
`tests/env_policy_lane_support.py`). Make, uv and the lane driver still run for
real, and the test judges exact ordered argument arrays, the named failure
message and visible diagnostics. The fake compiles nothing, so the contract can
no longer wait on a co-scheduled resolve.

Validated on 2026-11-09, with `flock -x ~/.cargo/.package-cache` holding the
lock: `cargo metadata --format-version 1` printed
`Blocking waiting for file lock on package cache` and the control timed out
after 3 seconds, while the contract module passed 7 tests in 1.02 seconds (1.76
seconds including `uv run` start-up). The control proves the lock was held for
the contract run.

The other options were considered and rejected:

- **`--offline`, `--locked` or `--frozen`**: Cargo documents these as network
  and lockfile limits, not as cache-lock avoidance. `--offline` forbids network
  access; `--locked` and `--frozen` require `Cargo.lock` to be unchanged. None
  promises that a resolve will not take the package-cache lock, so they would
  leave the dependency in place.
- **A larger test marker**: this keeps the contract dependent on another agent's
  build. It converts a policy contract into a measurement of host load, and a
  slow unrelated build can still exhaust any bound.
- **Requiring only that the lane *started* before a deadline**: this weakens
  the assertion from "the policy target propagates failure" to "the target
  began", which a broken driver could satisfy.

Real compilation and policy enforcement remain with `rust_extension/tests/`
(the probe and the `syn` source scan) and with `make lint`. The Makefile,
driver and CI wiring are unchanged.
