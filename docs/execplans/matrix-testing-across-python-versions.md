# Add Python-Version CI Matrix With 3.15a Allowed Failure

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE (implemented; local validation complete)

## Purpose / big picture

The CI workflow currently runs without an explicit Python-version matrix in
`.github/workflows/ci.yml`, so compatibility across supported interpreters is
not enforced per pull request. This change adds a matrix so tests and type
checking are required to pass on Python `3.12`, `3.13`, and `3.14`, while
Python `3.15` pre-release (3.15a lane) is executed as a non-blocking signal.

After this change, a pull request will show separate CI checks per Python
version, and maintainers will see early breakage against Python 3.15 alpha
without blocking merges.

## Constraints

- Modify only CI and documentation files for this task:
  - `.github/workflows/ci.yml`
  - `docs/dev-workflow.md` (if CI behaviour description changes)
- Do not change library/runtime code under `femtologging/`, `rust_extension/`,
  or `tests/`.
- Keep existing quality gates present in CI (`make check-fmt`, `make lint`,
  `make typecheck`, `make test`) unless explicitly replaced with equivalent
  coverage.
- Keep `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0` in CI env.
- Preserve pull-request trigger scope (`pull_request` on `main`) unless asked
  otherwise.

## Tolerances (exception triggers)

- Scope: if implementation requires edits outside the files listed in
  Constraints, stop and escalate.
- Workflow growth: if `ci.yml` net change exceeds 140 lines, stop and escalate
  with a simpler alternative.
- Runtime: if CI duration for required lanes (`3.12`/`3.13`/`3.14`) exceeds
  35 minutes median for 3 consecutive runs, escalate with optimisation options.
- Tooling: if `actions/setup-python` cannot resolve `3.15` pre-release with
  `allow-prereleases` enabled, stop and escalate with fallback choices.
- Ambiguity: if branch protection expectations conflict with allowed-failure
  semantics, stop and escalate.

## Risks

- Risk: matrix multiplies runtime and cache contention.
  Severity: medium Likelihood: medium Mitigation: keep `fail-fast: false` for
  visibility; include Python version in cache keys where applicable; optimise
  in follow-up only if needed.

- Risk: 3.15 pre-release resolver behaviour may change over time.
  Severity: medium Likelihood: medium Mitigation: use `python-version: "3.15"`
  with `allow-prereleases: true`, monitor lane regularly, and keep lane
  non-blocking.

- Risk: allowed-failure lane may be misread as required in branch protection.
  Severity: medium Likelihood: low Mitigation: document intent in workflow/job
  naming and repository docs.

## Progress

- [x] (2026-02-06 16:29Z) Reviewed existing CI workflow and Make targets.
- [x] (2026-02-06 16:29Z) Confirmed project has no `PLANS.md`.
- [x] (2026-02-06 16:29Z) Verified matrix `continue-on-error` pattern and
      `setup-python` pre-release support from GitHub docs.
- [x] (2026-02-07 01:30Z) Edited `.github/workflows/ci.yml` to add the
      Python-version matrix and the permitted-failure lane.
- [x] (2026-02-07 01:31Z) Updated `docs/dev-workflow.md` with CI matrix policy.
- [x] (2026-02-07 01:46Z) Ran local gates:
      `make fmt`, `make markdownlint`, `make nixie`, `make check-fmt`,
      `make lint`, `make typecheck`, and `make test`.
- [ ] Push branch and verify PR check behaviour across all matrix lanes.

## Surprises & discoveries

- Observation: `.github/workflows/ci.yml` currently has one job and does not
  pin/select Python explicitly. Evidence: current file contains Rust/uv setup
  and runs Make targets, but no `actions/setup-python` step. Impact: CI
  currently depends on runner default Python, so explicit version compatibility
  is not guaranteed.

- Observation: no `PLANS.md` exists in repository root.
  Evidence: file lookup returned “PLANS.md not found”. Impact: ExecPlan follows
  skill skeleton directly with no repo-specific plan wrapper requirements.

## Decision log

- Decision: use one matrix job in `ci.yml` with `include` entries for
  `3.12`, `3.13`, `3.14`, and `3.15` + `experimental` flag. Rationale: this
  gives precise control over required vs allowed-failure lanes. Date/Author:
  2026-02-06 / Codex.

- Decision: apply `continue-on-error: ${{ matrix.experimental }}` at job level.
  Rationale: this is the GitHub-supported mechanism for permitted-failure
  matrix entries. Date/Author: 2026-02-06 / Codex.

- Decision: set `allow-prereleases: ${{ matrix.experimental }}` in
  `actions/setup-python`. Rationale: ensures `3.15` resolves to pre-release
  builds while not affecting stable lanes. Date/Author: 2026-02-06 / Codex.

## Outcomes & retrospective

Implemented in repository with local validation complete. Required and
experimental matrix definitions are now configured in CI, and docs were
updated to match. External verification of PR check behaviour is still pending
until branch push and GitHub run execution.

## Context and orientation

Relevant files and current responsibilities:

- `.github/workflows/ci.yml`
  Current pull-request workflow. Today it runs one `build-test` job with Make
  targets: `check-fmt`, `lint`, `typecheck`, `test`.

- `Makefile`
  Defines quality gates and build/test commands used by CI.

- `pyproject.toml`
  Declares `requires-python = ">=3.12"`, aligning with required lanes.

- `pyrightconfig.json`
  Current type checker config (`pythonVersion: 3.13`) that should still be
  exercised under each matrix interpreter via `make typecheck`.

- `docs/dev-workflow.md`
  Developer-facing CI/workflow guidance that should reflect matrix policy.

Definitions used in this plan:

- Matrix job: one workflow job expanded into multiple parallel runs, one per
  Python version.
- Permitted failure: a matrix entry that may fail without failing the workflow,
  implemented via `continue-on-error: true`.

## Plan of work

### Stage A: CI matrix design (no runtime code changes)

Define matrix entries in `.github/workflows/ci.yml`:

- required rows:
  - `python-version: "3.12"`, `experimental: false`
  - `python-version: "3.13"`, `experimental: false`
  - `python-version: "3.14"`, `experimental: false`
- allowed-failure row:
  - `python-version: "3.15"`, `experimental: true`

Add `actions/setup-python` before `setup-uv` so each matrix row runs with a
known interpreter. Keep existing env and quality commands.

Go/no-go:

- Go if workflow remains valid YAML and keeps existing gate commands.
- No-go if this requires changing product code or test logic.

### Stage B: Implement workflow updates

Update `.github/workflows/ci.yml`:

- Add job-level:
  - `continue-on-error: ${{ matrix.experimental }}`
- Add strategy block:
  - `fail-fast: false`
  - matrix `include` rows listed above
- Add step:
  - `uses: actions/setup-python@v6`
  - `python-version: ${{ matrix.python-version }}`
  - `allow-prereleases: ${{ matrix.experimental }}`

Preserve existing Rust/cache/uv/tool setup and command steps.

Go/no-go:

- Go if required rows fail the workflow when broken and 3.15 row does not.
- No-go if allowed-failure behaviour cannot be observed in PR checks.

### Stage C: Documentation alignment

Update `docs/dev-workflow.md` with a short CI section:

- Required compatibility lanes: `3.12`, `3.13`, `3.14`
- Early-warning lane: `3.15` pre-release, non-blocking
- Note why pre-release lane is allowed failure

Go/no-go:

- Go if docs match actual workflow behaviour.
- No-go if docs become speculative or diverge from workflow file.

### Stage D: Validation and rollout

Validate syntax and local gates, then run CI on a branch/PR and inspect check
results.

Acceptance gate:

- `3.12`, `3.13`, and `3.14` matrix rows pass `make typecheck` and `make test`.
- `3.15` row runs and may fail without failing overall workflow.
- Workflow remains required/green when only 3.15 fails.

## Concrete steps

Run from repository root:

    set -o pipefail
    make check-fmt 2>&1 | tee /tmp/matrix-ci-check-fmt.log
    make lint 2>&1 | tee /tmp/matrix-ci-lint.log

After editing workflow and docs:

    set -o pipefail
    make fmt 2>&1 | tee /tmp/matrix-ci-fmt.log
    make markdownlint 2>&1 | tee /tmp/matrix-ci-markdownlint.log
    make typecheck 2>&1 | tee /tmp/matrix-ci-typecheck.log
    make test 2>&1 | tee /tmp/matrix-ci-test.log

Expected PR check shape after push:

    CI / build-test (3.12) -> pass
    CI / build-test (3.13) -> pass
    CI / build-test (3.14) -> pass
    CI / build-test (3.15, allowed failure) -> pass or fail (non-blocking)
    Overall workflow conclusion -> success when only 3.15 fails

## Validation and acceptance

Behavioural acceptance:

- A deliberate break in type checking must fail required lanes
  (`3.12`/`3.13`/`3.14`).
- A deliberate Python-3.15-only incompatibility must fail only the 3.15 lane
  and keep workflow green.
- Existing non-matrix quality behaviour remains intact (format/lint/test still
  executed by CI commands).

Quality criteria:

- Tests: `make test` passes on required matrix rows.
- Type checking: `make typecheck` passes on required matrix rows.
- Lint/format: existing CI lint/format commands still run and pass.
- Docs: `make markdownlint` passes after documentation update.

## Idempotence and recovery

- Editing `ci.yml` and `docs/dev-workflow.md` is idempotent and safe to rerun.
- If workflow config is invalid, revert only the latest CI/doc changes and
  reapply incrementally.
- If 3.15 setup fails due unavailable pre-release builds, keep lane
  non-blocking and escalate with alternatives (`3.15-dev` pin or temporary
  disable) rather than changing required lanes.

## Artifacts and notes

Proposed matrix fragment for `ci.yml` (illustrative):

    continue-on-error: ${{ matrix.experimental }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - python-version: "3.12"
            experimental: false
          - python-version: "3.13"
            experimental: false
          - python-version: "3.14"
            experimental: false
          - python-version: "3.15"
            experimental: true

    - name: Set up Python
      uses: actions/setup-python@v6
      with:
        python-version: ${{ matrix.python-version }}
        allow-prereleases: ${{ matrix.experimental }}

References used for design choices:

- GitHub docs: matrix failure handling
  <https://docs.github.com/en/actions/using-jobs/using-a-matrix-for-your-jobs>
- `actions/setup-python` docs (pre-release support)
  <https://github.com/actions/setup-python>
- `actions/setup-python` advanced usage (allow pre-releases)
  <https://github.com/actions/setup-python/blob/main/docs/advanced-usage.md>

## Interfaces and dependencies

- GitHub Actions workflow syntax:
  - `jobs.<job_id>.strategy.matrix`
  - `jobs.<job_id>.continue-on-error`
- GitHub Action:
  - `actions/setup-python@v6`
  - input: `python-version`
  - input: `allow-prereleases`
- Existing project commands:
  - `make check-fmt`
  - `make lint`
  - `make typecheck`
  - `make test`

Revision note (2026-02-06): initial draft created from current repository state
and official GitHub Actions references. No implementation has started.

Revision note (2026-02-07): implemented matrix testing in
`.github/workflows/ci.yml`, updated `docs/dev-workflow.md`, and completed local
validation gates.
