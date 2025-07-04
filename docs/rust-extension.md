# Rust Extension

This project bundles a Rust extension built with
[PyO3](https://pyo3.rs/). It started as a small experiment exposing a trivial
`hello()` function and a basic `FemtoLogger`. The extension now implements the
core logging components:

- `FemtoStreamHandler` writes log records to `stdout` or `stderr` on a
  background thread.
- `FemtoFileHandler` persists records to a file, also using a dedicated worker
  thread. It now provides `flush()` and `close()` to deterministically manage
  that thread.
- `FemtoLogger` spawns a background thread for experimentation. When dropped it
  sends a shutdown command over a `crossbeam-channel` so the thread exits even
  if additional `Sender` clones remain.

Packaging is handled by [maturin](https://maturin.rs/). The `[tool.maturin]`
section in `pyproject.toml` declares the extension module as
`femtologging._femtologging_rs`, so running `pip install .` automatically builds
the Rust code. Windows users may need the MSVC build tools installed or may need
to run maturin with `--compatibility windows`.

`FemtoLogRecord` now groups its contextual fields into a `RecordMetadata`
struct. Each record stores a timestamp, source file and line, module path and
thread ID. The thread name is included when available, along with any structured
key‑value pairs. Use `FemtoLogRecord::new` for default metadata or
`FemtoLogRecord::with_metadata` to supply explicit values.
