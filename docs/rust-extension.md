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

`FemtoLevel` defines the standard logging levels (`TRACE`, `DEBUG`, `INFO`,
`WARN`, `ERROR`, `CRITICAL`). Each `FemtoLogger` holds a current level and drops
messages below that threshold. The `set_level()` method updates the logger's
minimum level from Python or Rust code. The `log()` method returns the formatted
string or `None` when a message is filtered out.
