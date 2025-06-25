# Dependency Analysis

This note tracks third-party libraries required for the Rust port of
`picologging` and proposes Rust equivalents where appropriate.

## Python project

The current Python package has no runtime dependencies. Development tools are
`pytest`, `ruff` and `pyright` as configured in `pyproject.toml`. Static type
checking uses the `ty` CLI. A `Makefile` in the project root wraps these tools
with convenient targets (`fmt`, `check-fmt`, `lint`, `test`, `build` and
`release`). The `tools` target ensures commands like `ruff` and `ty` are
present.

## Rust ecosystem

The design document discusses several crates that map to parts of the CPython
implementation:

- **PyO3** provides bindings so the Rust library can be imported from Python. It
  replaces the C++ extension used by picologging.
- **crossbeam-channel** (v0.5.15) is recommended as the baseline synchronous
  multi-producer, single-consumer queue for handler threads. Alternatives like
`flume` or `tokio::sync::mpsc` may be benchmarked later. Version 0.5.15 avoids
the double-free vulnerability disclosed in RUSTSEC-2025-0024. The current
implementation uses a bounded channel with a capacity of 1024 messages so that
log producers cannot exhaust memory if the consumer thread stalls.
- **rstest** is used as a development dependency to provide concise test
  fixtures and parameterized tests.
- **serde** will power any structured data serialization needed for network
  handlers or configuration files. This crate is not yet listed in `Cargo.toml`
  because serialization features are planned for a later phase.
- **chrono** or `time` will supply timestamp utilities for `FemtoLogRecord`.
  These dependencies will be added once timestamp formatting is implemented.

These choices prioritize crates with strong community adoption and good
performance characteristics.
