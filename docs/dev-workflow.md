# Development Workflow

This project uses a `Makefile` to keep routine development tasks consistent
across Python and Rust code.

## Commands

- `make fmt` – format Python, Rust and Markdown sources.

- `make check-fmt` – verify formatting without modifying files.

- `make lint` – run `ruff check` and `cargo clippy` with
  `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0`.

- `make typecheck` – run

  ```shell
  ty check --extra-search-path=/root/.pyenv/versions/3.13.3/lib/python3.13/site-packages
  ```

  This target depends on `make build`.

- `make build` – compile the Rust extension by running `pip install -e .`.

- `make release` – build the extension with optimizations.

- `make clean` – remove build artifacts.

- `make tools` – verify required commands like `ruff` and `ty` are installed. In
  CI these tools are installed with `uv tool install` before formatting checks
  run.

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

ABI3 forward compatibility is disabled to simplify building for the currently
supported Python versions. Producing a library that worked across multiple
Python releases proved problematic; therefore,
`PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0` is set both locally and in CI.

These targets ensure style, type safety and correctness across the project.
