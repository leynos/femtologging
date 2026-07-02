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
pinned `uvx ruff==$(RUFF_VERSION)` command. Pull-request CI installs
`ruff==0.15.12` before running the same Makefile targets, so `make lint` and CI
evaluate Python lint rules with the same Ruff release.

When updating Ruff, first install the intended new version locally, confirm it
with `$(which ruff) --version`, then update both `RUFF_VERSION` in `Makefile`
and the `uv tool install ruff==…` line in the GitHub workflow
`.github/workflows/ci.yml`.

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
