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
minimum level using a `FemtoLevel` value. Likewise, `log()` accepts a
`FemtoLevel` and message, returning the formatted string or `None` when a record
is filtered out.

`FemtoLogger` can now dispatch a record to multiple handlers. Handlers implement
`FemtoHandlerTrait` and run their I/O on worker threads. A logger holds a
`Vec<Arc<dyn FemtoHandlerTrait>>`; calling `add_handler()` stores another
handler reference. When `log()` creates a `FemtoLogRecord`, it sends a clone to
each configured handler, ensuring thread‑safe routing via the handlers' MPSC
queues.

Each logger also spawns a small worker thread. Log calls send records over a
bounded `crossbeam-channel`, keeping the hot path non-blocking. Dropping the
logger sends a shutdown signal, so the worker can drain remaining records and
exit cleanly. A timeout warns if the thread does not finish within one second.

Currently, `add_handler()` is only available from Rust code. Python users still
create a logger with a single default handler. Support for attaching additional
handlers from Python will be added once the trait objects can be safely
transferred across the FFI boundary.
