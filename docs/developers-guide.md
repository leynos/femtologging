# Developers Guide

This guide records project-local development tool choices that must stay
consistent between local `make` targets and CI.

## Ruff version

Ruff is pinned to version `0.15.12`.

The pin was taken from the active local executable with:

```shell
$(which ruff) --version
```

which reported:

```text
ruff 0.15.12
```

The Makefile stores this value in `RUFF_VERSION` and invokes Ruff through the
pinned `uvx ruff==$(RUFF_VERSION)` command. Pull-request CI installs `uv` and
`ty`, then runs the same Makefile targets, so `make lint` and CI evaluate
Python lint rules with the same Ruff release through the Makefile.

When updating Ruff, first install the intended new version locally, confirm it
with `$(which ruff) --version`, then update `RUFF_VERSION` in `Makefile`. CI
does not install Ruff directly; it picks up the version from the Makefile's
`uvx ruff==$(RUFF_VERSION)` invocation.

## Python test toolchain

The Python test toolchain is pinned so local runs and CI use the same pytest
release:

- `pytest` is pinned to `8.4.2` in `pyproject.toml` and CI.
- `pytest-bdd` is pinned to `8.1.0` in `pyproject.toml` and CI.

Keep this policy in sync with `pyproject.toml` and the matching CI install
steps where applicable.

## Typos spelling checker

Markdown spelling is enforced with [`typos`](https://github.com/crate-ci/typos)
so that documentation stays in en-GB-oxendict (Oxford "-ize") spelling.

- `typos` is pinned to `1.48.0`. The pin lives in `TYPOS_VERSION` in the
  `Makefile`, which is the single source of truth; any CI that shells out to the
  `markdownlint` target reuses it, so the Makefile and CI cannot drift apart.
- The `make markdownlint` target runs
  `typos --config typos.toml --force-exclude` across the tracked Markdown files
  after `markdownlint-cli2`.

### Configuration

`typos.toml` is generated, not hand-edited. The `en-gb` locale corrects
American spellings to British ones but prefers the `-ise` family, so
`scripts/generate_typos_config.py` restores Oxford `-ize` spelling: for each
curated stem it emits an identity entry accepting the `-ize` inflection and an
`-ise` to `-ize` correction. Words that only ever take `-yse` (analyse,
paralyse) are left to the locale, and genuinely `-ise`-only words (advise,
revise, supervise) are excluded from the stem list so they remain accepted.

To add or change accepted vocabulary, edit the `STEMS` or
`EXTRA_ACCEPTED_WORDS` tuples in `scripts/generate_typos_config.py` and
regenerate the config rather than editing `typos.toml` by hand:

```shell
uv run scripts/generate_typos_config.py
```

When updating the pinned version, change `TYPOS_VERSION` in the `Makefile`,
update any matching CI install step, and re-run `make markdownlint` to confirm
the corpus still passes.

## Rust extension build toolchain

The Rust extension build toolchain is pinned so local builds, CI, and the
compatibility tests validate the same maturin and PyO3 releases:

- maturin is pinned to `1.13.3` in the development dependencies and CI build
  steps, with the build-system requirement bounded as `>=1.13.3,<2.0.0`.
- PyO3 is pinned to `0.28.3` in `rust_extension/Cargo.toml`.

When updating either dependency, change the pin in the source manifest, update
the matching CI install step where applicable, and run the maturin/PyO3
compatibility checks through the normal `make test` gate. The synchronization
test in `tests/test_maturin_build.py` verifies that the maturin pins stay
aligned across `pyproject.toml` and `.github/workflows/heavy-tests.yml`.

For `rust_extension/tests/compile_tests.rs`, use
`TRYBUILD=overwrite cargo test --test compile_tests` after rustc or PyO3
changes to refresh `invalid_pymodule_return.stderr` and the other `.stderr`
fixtures.

## Toolchain Boundaries

The root `Makefile` is the source of truth for local and CI tool commands. Keep
new developer tooling behind a Makefile variable or target so CI and local
commands exercise the same path.

Ruff is pinned by `RUFF_VERSION` in the `Makefile`. The `RUFF` variable invokes
`uvx ruff==$(RUFF_VERSION)`, so `make fmt`, `make check-fmt`, and `make lint`
resolve the same formatter and linter version without requiring a global Ruff
install. CI must not add a second hard-coded Ruff installation; update
`RUFF_VERSION` when the project intentionally changes Ruff releases.

The `ty` command remains an installed developer tool because `make typecheck`
calls it directly. CI installs `uv` and `ty`, then delegates formatting,
linting, type checking, and tests to Makefile targets.

## Benchmarking Documentation

Benchmarking work is governed by
[benchmarking-and-optimization-design.md](./benchmarking-and-optimization-design.md)
and tracked in [roadmap.md](./roadmap.md). Keep those links intact when
editing developer workflow notes so contributors can move from toolchain setup
to the benchmarking phase design without losing context.

## Validation

Before committing, run the gates requested by the change. For code changes, the
full local sequence is:

```shell
make check-fmt
make test
make typecheck
make lint
```

For documentation changes, also run:

```shell
make fmt
make markdownlint
make nixie
```

These commands mirror the PR gates described in
[dev-workflow.md](./dev-workflow.md) and keep CI behaviour aligned with local
validation.
