# Architectural decision record (ADR) 0009: main-branch coverage publication

## Status

Accepted — 2026-11-19: pull requests measure coverage against the stored
baseline, and only the default branch publishes the baseline and the CodeScene
report. Both runner-provider cache stores receive the baseline.

## Date

2026-11-19.

## Context and problem statement

CV-005 separates coverage measurement from coverage publication. Before it, a
pull-request lane both measured coverage and published its report, and the
ratchet had never been enabled on that lane. The CodeScene token is a
deployment secret: it lives in the protected `codescene` environment, whose
deployment policy admits `main` alone, so no pull-request-reachable job may
read it.

The ratchet baseline is stored in the Actions cache, and its key carries the
operating system and the measured interpreter's major.minor. Two runner
providers serve this repository: Ubicloud runs same-repository pull requests,
and GitHub-hosted runners serve forks, Dependabot pull requests and the
main-branch measurement. Each provider reads its own cache store. A baseline
written on one store is invisible to a lane running on the other, and a lane
that restores nothing compares against zero and can never fail.

CodeScene attributes a report to a branch and commit by reading the clone it
runs in. The upload therefore needs the measured revision, not whichever ref
the default checkout resolves to.

## Decision drivers

- A pull request must know whether it lowers coverage before it merges.
- Branch code must never be able to read the CodeScene token.
- The ratchet must be able to fail on every pool that runs pull-request lanes.
- A CodeScene outage must not stop the regression baseline advancing.
- A failed lane must not withdraw a successful sibling lane's publication.
- Publication must remain attributable to the commit that was measured.

## Options considered

1. **Publish from the pull-request lane.** This keeps one workflow, but it
   puts the token within reach of a branch's workflow change, lets unmerged
   code advance the trunk's baseline, and sends CodeScene a report for every
   pull-request revision.
2. **Measure only on the default branch.** This removes the token exposure
   entirely, but a pull request then merges without any coverage check of its
   own change.
3. **Publish the baseline from the GitHub-hosted measurement job.** This is
   the simplest publisher, but same-repository pull requests run on Ubicloud
   and read a different cache store, so their ratchet would restore nothing and
   could never fail.
4. **Measure on pull requests without publishing; publish from the default
   branch into both stores** (chosen). The pull-request lane keeps a ratchet
   check and publishes nothing, and `coverage-main.yml` owns both persistent
   outputs.

## Decision outcome

Adopt `.github/workflows/ci.yml` for measurement and
`.github/workflows/coverage-main.yml` for publication. Both call the pinned
`leynos/shared-actions` actions at commit `203eedc`.

The pull-request lane runs `generate-coverage` with `with-ratchet: 'true'`,
`publish-artefact: 'false'` and `cache-provider: external`, and its job
declares no environment. It neither uploads a report nor contacts CodeScene.

`coverage-main.yml` triggers on a push to `main` alone and has four jobs:

- `coverage-measure` measures once per required Python lane (3.12, 3.13 and
  3.14) on a GitHub-hosted runner. It passes `publish-baseline: never` so it
  neither restores nor saves a cache, and hands each lane's figures to its
  publisher through a `coverage-baseline-handover-<python>` artefact.
- `coverage-baseline` ratchets those figures and saves them to the cache store
  the Ubicloud pull-request lanes read.
- `coverage-baseline-hosted` ratchets the same figures and saves them to
  GitHub's cache store, which is what a fork or Dependabot pull request
  restores.
- `coverage-upload` runs in the `codescene` environment with only
  `contents: read`, checks out `${{ github.sha }}`, fetches the 3.14 report and
  uploads it. It runs on `!cancelled()`, so a failed sibling lane still lets a
  successful lane's report reach CodeScene, and a missing token skips the
  upload without failing the job: an upload outage cannot hide a coverage
  regression.

The publishers keep `fail-fast: false`, so one lane's ratchet failure does not
withdraw another's publication. Each publisher runs under `!cancelled()` and
publishes a lane only when that lane's hand-over is present. A missing
hand-over while every lane succeeded stays a failure, because that is a lost
publication rather than a lane that measured nothing.

`tests/test_coverage_environment_contract.py`,
`tests/test_coverage_upload_guard_contract.py`,
`tests/test_coverage_publisher_contract.py` and
`tests/test_coverage_reach_contract.py` hold the split to the files.

## Consequences

- A pull request fails its own check when it lowers coverage, without ever
  being able to read the token or write the baseline.
- A CodeScene outage, or an absent token, does not stop the baseline from
  advancing.
- The same figure is ratcheted twice, once per cache store; the duplication is
  the cost of serving two disjoint runner providers.
- CodeScene receives one report per measured commit, attributed to that
  commit.

## Known risks

- **Fail-open on a cache miss.** `generate-coverage` writes `0` into a
  baseline file its restore did not supply
  ([leynos/shared-actions#699](https://github.com/leynos/shared-actions/issues/699)).
  A lost cache entry therefore starts the baseline at zero, and that lane's
  ratchet cannot fail while the run still looks green. The pinned action
  reports its restore outcome (`metric ratchet-cache.restore=<hit|miss|…>` and a
  `generate-coverage ratchet cache` job-summary heading), and `ci.yml`
  additionally warns when the restored interpreter baseline is absent or zero,
  but neither repairs the gate; the fix belongs in the action.
- **Dependabot merge without a push event.** A Dependabot pull request merged
  by `dependabot-automerge` with `GITHUB_TOKEN` fires no push event, so that
  merge publishes nothing until the next push to `main`.
- **Two stores must stay in step.** A store written by neither publisher
  leaves the pool that reads it fail-open, which is why the hosted publisher
  exists even though the main-branch workflow cannot itself reach the hosted
  arm.
- **The 3.14 lane is the only report source.** Only that lane keeps its
  report, because CodeScene analyses one report per commit; a failure confined
  to that lane costs the report but not the baseline.

## References

- `.github/workflows/ci.yml` — pull-request measurement and ratchet.
- `.github/workflows/coverage-main.yml` — measurement, both publishers and the
  CodeScene upload.
- [leynos/shared-actions#699](https://github.com/leynos/shared-actions/issues/699)
  — zero-baseline fail-open exposure in `generate-coverage`.
- `docs/developers-guide.md`, section "Coverage publication".
