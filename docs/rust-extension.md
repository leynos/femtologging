# Rust Extension

This project includes a small Rust extension built with
[PyO3](https://pyo3.rs/). Initially, it exposed only a trivial `hello()`
function and the `FemtoLogger` class. It has since grown to provide the core
handler implementations as well:

- `FemtoStreamHandler` writes log records to `stdout` or `stderr` on a
  background thread.
- `FemtoFileHandler` persists records to a file, also using a dedicated worker
  thread. It now provides `flush()` and `close()` to deterministically manage
  that thread.
- `FemtoFileHandler` now supports a configurable overflow strategy (`Drop`,
  `Block`, or `Timeout`) via the `with_capacity_policy()` constructor.

Packaging is handled by [maturin](https://maturin.rs/).

The `[tool.maturin]` section in `pyproject.toml` declares the extension module
as `femtologging._femtologging_rs`, so running `pip install .` automatically
builds the Rust code. Windows users may require the MSVC build tools to be
installed, or may need to invoke maturin with `--compatibility windows`.

The extension also exposes an `OverflowPolicy` enum and the
`FemtoFileHandler::with_capacity_policy()` constructor; see
`docs/formatters-and-handlers-rust-port.md` for usage examples.
