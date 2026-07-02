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
