# Development Workflow

This project uses a `Makefile` to keep routine development tasks consistent
across Python and Rust code.

## Commands

- `make fmt` – format Python, Rust and Markdown sources.

- `make check-fmt` – verify formatting without modifying files.

- `make lint` – run the pinned Ruff checker and `cargo clippy` with
  `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0`.

- `make typecheck` – run the pinned `ty` command

  ```shell
  ty check --python ./.venv --extra-search-path scripts
  ```

  This target depends on `make build`. The explicit Python path selects the
  project virtual environment, and `scripts` makes the helper modules
  importable as top-level modules during type checking. The release is pinned
  by the Makefile; use this target rather than an independently installed
  `ty` version.

- `make build` – compile the Rust extension by running `pip install -e .`.

- `make release` – build the extension with optimizations.

- `make clean` – remove build artefacts.

- `make tools` – verify required commands like `uv`, `ty`, `cargo` and
  `rustfmt` are installed.

- `make test` – run formatting checks, clippy, cargo tests and pytest. This
  target depends on `make build`.

- `make markdownlint` – lint Markdown files.

- `make nixie` – validate Mermaid diagrams embedded in Markdown.

- `make help` – list available targets.

## CI compatibility matrix

Pull-request CI uses a Python-version matrix in `.github/workflows/ci.yml`:

- Required lanes: Python `3.12`, `3.13`, and `3.14`.
- Early warning lane: Python `3.15` pre-release as an allowed failure.

All lanes run the same gates: `make check-fmt`, `make lint`, `make typecheck`,
and `make test`.

Ruff is pinned in the root `Makefile` with `RUFF_VERSION`; the `RUFF` variable
uses `uvx ruff==$(RUFF_VERSION)` so formatting and linting use the same Ruff
release locally and in CI. The CI workflow installs `uv` and `ty`, then relies
on the Makefile targets to resolve the pinned Ruff version. Do not add a
separate hard-coded Ruff installation to CI; update `RUFF_VERSION` in the
Makefile when the project intentionally changes Ruff versions.

ABI3 forward compatibility is disabled to simplify building for the currently
supported Python versions. Producing a library that worked across multiple
Python releases proved problematic; therefore,
`PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0` is set both locally and in CI.

These targets ensure style, type safety and correctness across the project.
