# Rust Extension

This project includes a small Rust extension built with
[PyO3](https://pyo3.rs/) (currently `^0.25.1`). Initially, it exposed only a
trivial `hello()` function and the `FemtoLogger` class. It has since grown to
provide the core handler implementations as well:

- `FemtoStreamHandler` writes log records to `stdout` or `stderr` on a
  background thread.
- `FemtoFileHandler` persists records to a file, also using a dedicated worker
  thread. It now provides `flush()` and `close()` to deterministically manage
  that thread.

The file handler lives under `rust_extension/src/handlers/file`. This directory
splits responsibilities into three modules:

1. `config.rs` – configuration types shared with the Python bindings.
2. `worker.rs` — the asynchronous consumer thread that writes log records.
3. `mod.rs` — the public API exposing `FemtoFileHandler` and re‑exporting the
    configuration items.

```rust
use femtologging_rs::handlers::file::{FemtoFileHandler, HandlerConfig};
```

Packaging is handled by [maturin](https://maturin.rs/). Use version
`>=1.9.1,<2.0.0` as declared in `pyproject.toml`. The `[tool.maturin]` section
declares the extension module as `femtologging._femtologging_rs`, so running
`pip install .` automatically builds the Rust code. Windows users may need the
MSVC build tools installed, or may need to run maturin with
`--compatibility windows` to build.

PyO3 0.25 introduced `Bound` return types for constructors such as
`PyDict::new(py)`. When dictionaries must be returned to Python, use
`pyo3::IntoPyObjectExt::into_py_any(d, py)` rather than the pre‑0.25 pattern of
`unbind().into()`. This keeps the object bound to the Global Interpreter Lock
(GIL) during conversion.

```rust
let d = pyo3::types::PyDict::new(py);
let obj = pyo3::IntoPyObjectExt::into_py_any(d, py)?;
```

`FemtoLogRecord` now groups its contextual fields into a `RecordMetadata`
struct. Each record stores a timestamp, source file and line, module path and
thread ID. The thread name is included when available, along with any
structured key‑value pairs. Use `FemtoLogRecord::new` for default metadata or
`FemtoLogRecord::with_metadata` to supply explicit values.

`FemtoLevel` defines the standard logging levels (`TRACE`, `DEBUG`, `INFO`,
`WARN`, `ERROR`, `CRITICAL`). Each `FemtoLogger` holds a current level and
drops messages below that threshold. The `set_level()` method updates the
logger's minimum level using a `FemtoLevel` value. Likewise, `log()` accepts a
`FemtoLevel` and message, returning the formatted string or `None` when a
record is filtered out.

`FemtoLogger` can now dispatch a record to multiple handlers. Handlers
implement `FemtoHandlerTrait` and run their I/O on worker threads. The logger
keeps its handler list inside an `RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>`,
allowing handlers to be added through `&self`. Calling `add_handler()` pushes
another reference into that list. When `log()` creates a `FemtoLogRecord`, it
sends a clone to each configured handler, ensuring thread‑safe routing via the
handlers' MPSC queues.

Handlers observe log records in the order they are attached. Adding a handler
while other threads are logging is safe—new handlers only receive records
logged after the addition completes. The `remove_handler()` method works
through `&self` as well and detaches a handler so it no longer receives
subsequent records.

Handlers manage worker threads; the logger simply forwards each record to every
handler. Callers may invoke a handler's `flush()` method to ensure queued
messages are written before dropping it.

The `add_handler()` method is exposed through the Python bindings and can be
called on a shared logger instance. It verifies the provided object defines a
callable `handle(logger, level, message)` method and raises `TypeError` if the
check fails. Built-in handlers like `FemtoStreamHandler` and `FemtoFileHandler`
pass automatically; custom classes must implement a compatible `handle` method.
