# Update maturin and PyO3 compatibility validation

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

## Purpose / big picture

This branch upgrades the Python extension build toolchain to maturin 1.13.3 and
PyO3 0.28.3, then adds tests that make future upgrades observable. A maintainer
should be able to run the normal project gates and see that the installed
maturin version matches the pinned development dependency, the built wheel
still contains the expected metadata and package layout, and the PyO3
module/signature patterns used by the Rust extension still compile.

The implementation follows the approach used by Cuprum at commit
`df25f6c09e388cba1a055d167a5a88d13a8826fd`, adapted to femtologging's
repository layout and CI files.

## Constraints

- Keep the current branch `chore/maturin-pyo3-upgrade-tests`; do not work on a
  main branch.
- Use the repository Makefile targets for validation where available.
- Do not create an isolated Cargo cache; use the shared default Cargo cache.
- Do not use `/tmp` as a build target. Temporary logs and the reference Cuprum
  checkout may live in `/tmp`.
- Commit only changes that pass the requested gates:
  `make check-fmt`, `make lint`, `make typecheck`, and `make test`.
- Keep file edits focused on dependency pins and compatibility/build tests.
- Preserve public Python and Rust runtime behaviour; this work validates the
  build boundary and must not change logging semantics.

## Tolerances (exception triggers)

- Scope: if the implementation requires changing more than 12 tracked files,
  stop and document why before proceeding.
- Interface: if a public Python API, Rust public API, or wheel module name must
  change, stop and ask for direction.
- Dependencies: adding `trybuild` as a Rust development dependency is expected;
  any further external dependency requires a decision log entry before use.
- Iterations: if the same validation gate fails three times for unrelated root
  causes, stop and record the options.
- Ambiguity: if Cuprum's approach depends on repository files that femtologging
  lacks, adapt narrowly to the nearest femtologging equivalent and record the
  adaptation.

## Risks

- Risk: The wheel snapshot may include platform-specific filenames or maturin
  metadata that changes across Python versions. Severity: medium. Likelihood:
  medium. Mitigation: normalize extension module names, dist-info paths, SBOM
  entries, and wheel tags before snapshot comparison.

- Risk: A full maturin release build inside `make test` may increase runtime.
  Severity: medium. Likelihood: medium. Mitigation: reuse the existing test
  environment and build into a pytest temporary directory; the test is valuable
  because it catches packaging drift.

- Risk: PyO3 compile-fail diagnostics can change wording between patch
  releases. Severity: low. Likelihood: medium. Mitigation: import the Cuprum
  `trybuild` pattern, but accept that the stderr fixture may need deliberate
  updates during future PyO3 upgrades.

## Progress

- [x] 2026-06-05T01:35:56+02:00 Loaded requested `leta`,
  `python-router`, `rust-router`, and `hexagonal-architecture` skills.
- [x] 2026-06-05T01:35:56+02:00 Created a Leta workspace for the repository.
- [x] 2026-06-05T01:35:56+02:00 Confirmed the branch is
  `chore/maturin-pyo3-upgrade-tests`, not a main branch.
- [x] 2026-06-05T01:35:56+02:00 Inspected Cuprum commit
  `df25f6c09e388cba1a055d167a5a88d13a8826fd`.
- [x] 2026-06-05T01:43:57+02:00 Updated maturin and PyO3 dependency pins.
- [x] 2026-06-05T01:43:57+02:00 Added adapted maturin build
  compatibility helpers and tests.
- [x] 2026-06-05T01:43:57+02:00 Added PyO3 compile tests for module and
  signature compatibility.
- [x] 2026-06-05T01:43:57+02:00 Ran and fixed `make check-fmt`.
- [x] 2026-06-05T01:43:57+02:00 Ran and fixed `make lint`.
- [x] 2026-06-05T01:43:57+02:00 Ran and fixed `make typecheck`.
- [x] 2026-06-05T01:56:29+02:00 Ran and fixed `make test`.
- [x] 2026-06-05T01:56:29+02:00 Commit the gated change.
- [x] 2026-06-05T01:58:51+02:00 Created draft pull request
  <https://github.com/leynos/femtologging/pull/375>.
- [x] 2026-06-23T00:00:00+02:00 Addressed review feedback by making
  `toolchain_available()` probe `python -m maturin --version` end to end.
- [x] 2026-06-23T00:00:00+02:00 Fixed pytest 9 private entrypoint
  traceback normalization encountered while rerunning `make test`.
- [x] 2026-06-23T00:00:00+02:00 Reran and passed `make fmt`,
  `make markdownlint`, `make check-fmt`, `make lint`, `make typecheck`,
  `make test`, and `git diff --check`.
- [x] 2026-06-23T00:00:00+02:00 Fixed the post-turn `nixie` failure in
  `docs/frame-filtering-design.md` by quoting multi-line Mermaid node labels.
- [x] 2026-06-24T00:00:00+02:00 Pinned local Makefile Ruff execution and CI
  installation to Ruff `0.15.12`, matching `$(which ruff) --version`.

## Surprises & Discoveries

- Femtologging already has a checked-in `rust_extension/Cargo.lock`, but no
  repository-root `uv.lock`.
- Femtologging has no `.github/actions/build-wheels/action.yml`; Cuprum's
  maturin pin synchronisation test must therefore compare `pyproject.toml`
  dev/build-system pins and CI workflow usage instead of an absent reusable
  build-wheel action.
- Cuprum's dependency versions at the referenced commit match the latest
  versions discovered locally: maturin 1.13.3 and PyO3 0.28.3.
- `ty check` reported an existing redundant cast in
  `femtologging/adapter.py`; removing it made the typecheck gate clean rather
  than merely exit-code green.
- `logging.Formatter` formats `asctime` using local time by default, so the
  stdlib adapter timestamp test needed to derive its expected prefix with
  `time.localtime` rather than assuming UTC.
- Running `make fmt` after documentation changes normalized existing Markdown
  wrapping across several documents. `markdownlint` then exposed two existing
  long-line cases, one of which required a narrowly scoped MD013 suppression
  for an unbreakable roadmap link.
- `importlib.util.find_spec("maturin")` can succeed even when
  `python -m maturin` fails because the maturin Rust script is unavailable. The
  compatibility skip guard must therefore run the same module entry point that
  the wheel-build helper depends on.
- Pytest 9 can render launcher frames with `_console_main` and `_main` helper
  names. Snapshot normalization must canonicalize only the entrypoint helper
  frames so later internal `_main` frames remain visible.
- `nixie` rejects raw newline-delimited Mermaid flowchart labels. Quoted labels
  with explicit `<br/>` breaks preserve the rendered text while keeping
  `merman-cli` parsing stable.
- Bare `ruff` in local Make targets and unpinned `uv tool install ruff` in CI
  can drift independently. `uvx ruff==$(RUFF_VERSION)` keeps Makefile execution
  pinned without depending on a globally installed Ruff.

## Decision Log

- Decision: Pin the development maturin dependency exactly to `1.13.3` while
  keeping the build-system requirement bounded as `maturin>=1.13.3,<2.0.0`.
  Rationale: Cuprum pins the developer/test tool to make build metadata
  snapshots deterministic, while the PEP 517 build-system range remains
  compatible with future maturin 1.x releases.

- Decision: Adapt Cuprum's missing build-wheel action check to femtologging's
  actual CI workflows. Rationale: the requested approach is about synchronized
  build validation, not creating unrelated GitHub Action structure.

- Decision: Add `trybuild` as a Rust dev-dependency for PyO3 compile UI tests.
  Rationale: it validates macro compatibility at compile time and matches the
  Cuprum upgrade approach.

- Decision: Remove the redundant `typing.cast` in `femtologging/adapter.py`.
  Rationale: the requested gates should have no type-safety diagnostics, and
  the warning was unrelated but legitimate cleanup.

- Decision: Keep the Markdown formatting produced by `make fmt`. Rationale:
  the repository instructions require running `make fmt` after documentation
  changes, and the resulting edits are mechanical formatting updates required
  by the documented workflow.

## Implementation Plan

First, update `pyproject.toml` so the development environment installs
`maturin[patchelf]==1.13.3` and the build-system requires
`maturin>=1.13.3,<2.0.0`. Update workflow references that explicitly install
maturin so they use the same supported floor.

Second, update `rust_extension/Cargo.toml` from PyO3 0.28.0 to 0.28.3 and add
`trybuild` to `[dev-dependencies]`. Refresh `rust_extension/Cargo.lock` through
Cargo rather than manual editing.

Third, add `tests/maturin_compat.py` or an equivalent helper module adapted
from Cuprum. It should read maturin pins, build one native wheel with the
active maturin module, normalize volatile wheel entries, and return metadata
for snapshot assertion.

Fourth, add Python tests under `tests/` that verify synchronized maturin pins,
the installed maturin version, and normalized wheel output. The wheel test may
skip when the Rust toolchain or maturin module is unavailable, and it must skip
future unsupported Python interpreters where maturin cannot yet build.

Fifth, add Rust `trybuild` UI tests under `rust_extension/tests/` for the PyO3
module and signature patterns used by this crate. Include one compile-fail case
that guards the expected `#[pymodule]` return contract.

Finally, run `make check-fmt`, `make lint`, `make typecheck`, and `make test`
sequentially with `tee` logs in `/tmp`, fix every issue encountered, commit the
passing change, and create a draft pull request using the PR creation rules.

## Outcomes & Retrospective

The implementation is complete and has passed the requested validation gates:

- `make check-fmt`
- `make lint`
- `make typecheck`
- `make test`

Additional documentation validation also passed after the execplan and Markdown
formatting changes:

- `make fmt`
- `make markdownlint`

The branch now pins maturin and PyO3 to the upgraded versions, adds adapted
maturin wheel compatibility tests, adds PyO3 `trybuild` compile tests, and
cleans up gate failures encountered during validation.

The implementation was committed as `39d80dcbdc521e3022f713535694a0a7427cb21c`
and opened as draft pull request
<https://github.com/leynos/femtologging/pull/375>.

Review feedback on 2026-06-23 replaced the import-only maturin availability
check with a live `python -m maturin --version` probe so the skip predicate
matches the build path used by the compatibility test.

The validation rerun also exposed pytest 9 launcher-frame drift in the
stack-info BDD snapshot. The normalizer now maps the private entrypoint helper
spelling to the existing public `console_main`/`main` snapshot shape while
preserving subsequent internal pytest `_main` frames.
