# Development Workflow

This project uses a `Makefile` to keep routine development tasks consistent
across Python and Rust code.

## Commands

- `make fmt` – format Python, Rust and Markdown sources.
- `make check-fmt` – verify formatting without modifying files.
- `make lint` – run `ruff check` and `cargo clippy` with
  `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1`.
- `make typecheck` – run
  `ty check --extra-search-path=/root/.pyenv/versions/3.13.3/lib/python3.13/site-packages`.
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

These targets ensure style, type safety and correctness across the project.
