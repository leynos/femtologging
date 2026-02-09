# femtologging

**femtologging** is an experiment in building a fast, thread-friendly logging
library for Python using Rust. The project ports core ideas from [picologging]
(<https://github.com/microsoft/picologging>) and exposes them through a small
[PyO3](https://pyo3.rs/) extension. Log records travel over `crossbeam-channel`
queues to dedicated worker threads, ensuring the application remains responsive
even when log output is slow.

The goals are:

- CPython logging API compatibility
- strong compile-time guarantees from Rust
- minimal overhead through a producer–consumer model

For a deeper dive into the architecture and the crates involved, see the
documents in [`docs/`](./docs), especially

<!-- markdownlint-disable-next-line MD013 -->

[`rust-multithreaded-logging-framework-for-python-design.md`](docs/rust- multithreaded-logging-framework-for-python-design.md)
 and [`dependency- analysis.md`](docs/dependency-analysis.md).

## Installation

Ensure the [Rust toolchain](https://www.rust-lang.org/tools/install) is
available, then run:

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

# Attach a second handler
from femtologging import FemtoStreamHandler

log.add_handler(FemtoStreamHandler.stdout())

# Handlers can be added or removed at any time, even when the logger is
# shared. Newly added handlers only see subsequent records.

# Attach a custom Python handler
class Collector:
    def __init__(self) -> None:
        self.records: list[tuple[str, str, str]] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        self.records.append((logger, level, message))

collector = Collector()
log.add_handler(collector)
log.remove_handler(collector)
```

`FemtoStreamHandler` and `FemtoFileHandler` are available for basic output.
Each runs its I/O in a separate thread, so logging calls return immediately.

## Journald and OpenTelemetry status

Planned Journald and OpenTelemetry integrations are explicit opt-in features.
No application sends records to Journald or OpenTelemetry unless you enable the
relevant feature and configure the corresponding handler/layer yourself.

- Journald support is Linux/systemd-specific and is unavailable on Windows and
  macOS. On non-systemd Unix environments, use stream/file/socket handlers (or
  route logs to a local syslog/collector endpoint).
- Treat external observability sinks as potentially sensitive destinations:
  review message text and contextual keys before forwarding logs.
- Until the structured logging milestones in Phase 3 land, planned Journald and
  OpenTelemetry outputs are limited to message text plus basic metadata (for
  example logger name and level).

## Development

The [`Makefile`](./Makefile) defines tasks for formatting, linting, type
checking and tests. See [`docs/dev-workflow.md`](docs/dev-workflow.md) for
details.
