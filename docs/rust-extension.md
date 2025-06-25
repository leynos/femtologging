# Rust Extension

This variant includes a small Rust extension built with
[PyO3](https://pyo3.rs/). The extension exposes a `hello()` function and the
`FemtoLogger` class implemented in Rust. Packaging is handled by
[maturin](https://maturin.rs/), which is configured via `pyproject.toml` so that
`pip install .` builds the extension automatically.
Windows users may need the MSVC build tools installed or to run maturin with
`--compatibility windows`.
