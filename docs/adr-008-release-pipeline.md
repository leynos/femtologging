# Architectural decision record (ADR) 008: attested, reconciled release pipeline

## Status

Accepted.

## Date

2026-10-26.

## Context and problem statement

`femtologging` is a mixed Python and Rust package built with maturin. It needs
a release process that publishes one source distribution and every supported
native wheel to PyPI, attests the built files, and mirrors those same bytes to
a GitHub Release. The first release has not yet been published, so the workflow
and package configuration must be in place before creating its first tag.

The existing build configuration enabled `test-util` persistently so coverage
could exercise timed-rotation hooks. That configuration also applies to
ordinary maturin builds, which would put test-only APIs in production wheels.
The project therefore needs a production feature set alongside scoped coverage,
development and heavy-test overrides.

Reruns also need explicit handling. Native wheels rebuilt under the same
versioned filenames are rarely byte-identical, PyPI rejects a second upload of
an existing filename, and a GitHub Release asset must not be replaced after a
consumer may have downloaded it. A partial release must resume safely without
changing bytes already published at either destination.

## Decision drivers

- Build a native wheel matrix and exactly one sdist on every release build.
- Fail pull-request CI when the sdist is missing or duplicated.
- Keep test-only hooks out of production packages while retaining them for
  coverage and development tests.
- Attest the wheel and sdist files and publish through PyPI Trusted Publishing.
- Make a rerun of the same tag idempotent without overwriting existing files.
- Publish the GitHub Release only after its distribution assets match PyPI.
- Give each workflow job only the permissions it needs.

## Options considered

1. **Publish only from a local build.** This avoids workflow complexity, but
   leaves the build, attestation and release process dependent on a
   maintainer's machine and does not verify the sdist in pull-request CI.
2. **Upload every rebuilt file and replace GitHub assets on reruns.** This is
   simple, but the replacement can change bytes under a published filename and
   invalidate consumers' earlier digest or attestation checks.
3. **Build, attest and reconcile each filename across both destinations**
   (chosen). Existing bytes have a fixed precedence, destinations receive only
   names they lack, and publication stops if existing destinations disagree.

## Decision outcome

Adopt the workflow in `.github/workflows/release.yml` and its reusable
`.github/workflows/build-wheels.yml` workflow. The release workflow triggers on
`v*.*.*` tag pushes, sets top-level `permissions: {}`, grants permissions per
job, disables cancellation for a queued rerun of the same tag, and sets
`persist-credentials: false` on every checkout.

The reusable build produces CPython 3.12, 3.13 and 3.14 wheels for Linux x86_64
and aarch64, macOS x86_64 and arm64, and Windows x86_64. Wheels are
per-interpreter rather than ABI3. The Linux x86_64 leg also builds exactly one
sdist through the maturin backend. Pull-request CI calls this workflow;
`verify-wheel-install` checks sdist cardinality, inspects the sdist contents,
installs a native wheel and installs a rebuilt sdist into fresh environments.
Both isolated sdist builds constrain maturin to the workflow's
`MATURIN_VERSION`, keeping their backend aligned with the wheel builder. Python
3.15, free-threaded and Windows arm64 wheels are not in this matrix.

Linux builds and wheel verification use the existing `ubicloud-standard-4` lane
for pushes, same-repository pull requests and release tags. Fork and Dependabot
pull requests use `ubuntu-latest` instead. This keeps trusted build and
verification runs on the project's established Linux lane, while avoiding
execution of untrusted pull-request code on Ubicloud. The release build
therefore trusts that runner service to produce the uploaded files; the hosted
attestation job records the artefact digests but does not independently prove
the build runner's integrity.

The workflow phases are:

- `check-version` compares the tag with `rust_extension/Cargo.toml` and
  `pyproject.toml`, and reports whether the version is a pre-release. The Cargo
  crate remains `publish = false` and is shipped only within the wheel.
- `build` calls the reusable native wheel and sdist build.
- `attest` collects every wheel and exactly one sdist, generates build
  provenance for those files, and carries them with a run-unique Sigstore
  bundle in one `release-dist` artefact.
- `draft-release` creates or reuses the tag's draft GitHub Release and carries
  its current assets forward for reconciliation.
- `publish-pypi` runs in the `pypi` environment with only `contents: read` and
  `id-token: write`. It reads the PyPI Simple index, stages filenames PyPI does
  not already hold, then uses `pypa/gh-action-pypi-publish` with
  `skip-existing: true` under Trusted Publishing.
- `publish-release` uses PyPI's bytes for any name PyPI already holds, uploads
  only missing release assets, verifies matching SHA-256 digests across both
  destinations, attaches the provenance bundle, and then publishes the draft.
  It marks a release as a pre-release when `check-version` reports one.

### Reconciliation and reruns

For each distribution filename, existing PyPI bytes are canonical first,
existing GitHub Release bytes second, and the current build third. Only missing
names are staged for upload. The workflow uses no `--clobber` option. A rerun
of a complete release therefore uploads no new distribution files and succeeds
after verifying both destinations.

If PyPI and GitHub already hold different bytes under the same filename,
verification fails and the release remains a draft. Recovery is manual. Remove
the inconsistent GitHub Release asset because PyPI does not allow published
files to be deleted. Then rerun the same tag.

### Production and test feature sets

The Cargo defaults remain unchanged:

```toml
default = ["extension-module", "python", "log-compat", "tracing-compat"]
```

`test-util` is a separate, non-default feature. The persistent
`[tool.maturin].features = ["python"]` setting is the production package
configuration, so release wheels and the sdist exclude `test-util`. The probe in
`scripts/release_wheel_probe.py` checks that production wheels omit the
timed-rotation test hooks.

Coverage enables `test-util` through `.github/uv-test-util.toml`, selected with
an absolute `UV_CONFIG_FILE`. Development and heavy-test builds explicitly pass
`--features python,test-util`; `FEMTOLOGGING_EXPECT_TEST_UTIL=1` makes the test
suite fail early when those hooks are missing. Fresh-failure hooks remain a
known ungated test-only surface.

### Manual publishing settings

Before the first tag, maintainers must create the GitHub `pypi` environment and
restrict it to `v*.*.*` tags. They must also register a PyPI Trusted Publisher
with project `femtologging`, owner `leynos`, repository `femtologging`, workflow
`release.yml` and environment `pypi`. The project and publisher are not
registered by this repository change; the first release cannot publish until
those external settings exist.

## Consequences

- Pull requests verify that the build includes exactly one sdist and that
  both a wheel and rebuilt sdist install without test-only timed-rotation hooks.
- PyPI receives the production distributions through OIDC Trusted Publishing,
  and the GitHub Release becomes public only after cross-destination digest
  verification.
- Per-tag runs queue instead of cancelling an upload already in progress.
- Release telemetry is omitted; this repository has no release telemetry sink.
- The first release requires manual GitHub and PyPI publisher setup.

## Known risks

- A run-unique bundle records the attestation generated by that run, but it
  does not prove that carried-over bytes from PyPI or GitHub match the
  attestation subjects in that bundle.
- Existing PyPI/GitHub disagreement requires manual cleanup. PyPI files cannot
  be removed, so an inconsistent GitHub Release asset must be deleted before a
  rerun can converge.
- The release-wheel probe covers timed-rotation hooks. Fresh-failure test
  hooks remain a known ungated residual surface.
- The Ubicloud Linux lane is part of the release supply-chain trust boundary
  and an availability dependency. A compromised runner could alter an artefact
  before the hosted attestation job records its digest.
- The release workflow is a single point of failure for publication
  correctness; contract tests and scratch-directory reconciliation tests cover
  the checked-in workflow steps and helpers.
