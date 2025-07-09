# femtologging

**femtologging** is an experiment in building a fast, thread-friendly logging
library for Python using Rust. The project ports core ideas from
[picologging](https://github.com/microsoft/picologging) and exposes them through
a small [PyO3](https://pyo3.rs/) extension. Log records travel over
`crossbeam-channel` queues to dedicated worker threads, ensuring the application
remains responsive even when log output is slow.

The goals are:

- CPython logging API compatibility
- strong compile-time guarantees from Rust
- minimal overhead through a producer–consumer model

For a deeper dive into the architecture and the crates involved, see the
documents in [`docs/`](./docs), especially
[`rust-multithreaded-logging-framework-for-python-design.md`](docs/rust-multithreaded-logging-framework-for-python-design.md)
and [`dependency-analysis.md`](docs/dependency-analysis.md).

## Installation

Ensure the
[Rust toolchain](https://www.rust-lang.org/tools/install) is available, then
run:

```bash
pip install .
```

This compiles the extension with [maturin](https://maturin.rs/). Alternatively,
running `make build` yields the same result.

## Quick example

```python
from femtologging import get_logger

log = get_logger("demo")
log.log("INFO", "hello from femtologging")
```

`FemtoStreamHandler` and `FemtoFileHandler` are available for basic output. Each
runs its I/O in a separate thread, so logging calls return immediately.

## Development

The [`Makefile`](./Makefile) defines tasks for formatting, linting, type
checking and tests. See [`docs/dev-workflow.md`](docs/dev-workflow.md) for
details.
