# Dependency Analysis

This note tracks third-party libraries required for the Rust port of
`picologging` and proposes Rust equivalents where appropriate.

## Python project

The current Python package has no runtime dependencies. Development
tools are `pytest`, `ruff` and `pyright` as configured in
`pyproject.toml`.

## Rust ecosystem

The design document discusses several crates that map to parts of the
CPython implementation:

- **PyO3** provides bindings so the Rust library can be imported from
  Python. It replaces the C++ extension used by picologging.
- **crossbeam-channel** is recommended as the baseline synchronous
  multi-producer, single-consumer queue for handler threads. Alternatives
  like `flume` or `tokio::sync::mpsc` may be benchmarked later.
- **serde** will power any structured data serialization needed for
  network handlers or configuration files. This crate is not yet listed in
  `Cargo.toml` because serialization features are planned for a later phase.
- **chrono** or `time` will supply timestamp utilities for
  `FemtoLogRecord`. These dependencies will be added once timestamp
  formatting is implemented.

These choices prioritise crates with strong community adoption and good
performance characteristics.
