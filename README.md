# 🪵 femtologging

[![Ask DeepWiki][dw]][deepwiki] [![PyPI Version][pypi]][package]

[dw]: https://deepwiki.com/badge.svg
[deepwiki]: https://deepwiki.com/leynos/femtologging
[pypi]: https://img.shields.io/pypi/v/femtologging "PyPI package"
[package]: https://pypi.org/project/femtologging/

*Python logging that never waits for the disk: the API you know, with Rust
threads doing the writing.*

Femtologging gives Python applications familiar loggers whose records are
formatted and written by Rust worker threads. A log call queues a record and
returns, so slow terminals, files, and networks stay out of your hot path.

Website: <https://df12.studio/femtologging>

______________________________________________________________________

## Why femtologging?

If you have ever watched a busy service slow down because it was logging, you
know the trade-off: more detail, more waiting. Femtologging moves the waiting
somewhere else.

- **Calls return immediately**: Records go onto a bounded queue, and Rust
  worker threads handle formatting and I/O.
- **Familiar from the first line**: `get_logger()` (or `getLogger()`),
  `info()`, `exception()`, `basicConfig()`, `dictConfig()`, and `fileConfig()`
  work much as they do in the standard library.
- **Structured context built in**: Attach fields per call with `extra`, or for
  a whole block with `log_context()`, which is safe across `await`.
- **Bring your own handlers**: Plain Python objects can be handlers, and
  `StdlibHandlerAdapter` reuses standard `logging` handlers as they are.
- **Honest under pressure**: Full queues drop and count records instead of
  growing without limit or stalling your threads.

______________________________________________________________________

## Quick start

### Installation

Femtologging needs Python 3.12 or newer.

This README and the guides describe femtologging 0.2.0, which is currently in
beta. Installers skip pre-releases unless asked, so request the beta explicitly
with pip:

<!-- tested-example: readme-install-pip -->

```bash
python -m pip install "femtologging>=0.2.0b1"
```

Or add it to a [uv](https://docs.astral.sh/uv/) project:

<!-- tested-example: readme-install-uv -->

```bash
uv add "femtologging>=0.2.0b1"
```

The same requirement keeps working once 0.2.0 itself is released. Where no
pre-built wheel fits your platform, the installer compiles the extension from
source, which needs the
[Rust toolchain](https://www.rust-lang.org/tools/install).

### Quick taste

<!-- tested-example: readme-quick-start -->

```python
from femtologging import FemtoStreamHandler, get_logger

logger = get_logger("demo")
handler = FemtoStreamHandler.stdout()
logger.add_handler(handler)

message = logger.info("hello from femtologging")
assert message == "demo [INFO] hello from femtologging"

assert logger.flush_handlers()  # let the worker threads catch up
handler.close()
```

It prints:

<!-- tested-example: readme-quick-start-output -->

```text
demo [INFO] hello from femtologging
```

`logger.info()` queued the record and returned its rendered text at once; a
worker thread wrote it to standard output. Because the writing happens in the
background, flush and close handlers before your program exits.

### Where next?

- **New to femtologging?** The [users' guide][guide] picks up where this
  example stops: loggers, context, handlers, configuration, and running in
  production.
- **Coming from `logging`?** Keep
  [Differences from the standard library][guide-differences] to hand; a few
  behaviours differ on purpose.
- **Upgrading from 0.1?** The [0.2.0 migration guide][migration] lists the
  changes that can need an edit first.

______________________________________________________________________

## Features

- **Stream, file, and rotating file handlers**, rotating by size or on a
  schedule ([send records somewhere][guide-handlers]).
- **Socket and HTTP handlers** with background reconnection, backoff, TLS,
  and JSON output ([send records over a socket][guide-socket]).
- **Structured fields** per call or per scope, propagated through `asyncio`
  tasks via `contextvars` ([add context to records][guide-context]).
- **Exception and stack capture** with `exc_info`, `stack_info`, and
  `exception()` ([record exceptions][guide-exceptions]).
- **Python handlers and filters**, including enrichment filters and reuse of
  standard `logging` handlers ([write your own handler][guide-python-handler]).
- **Configuration your way**: `basicConfig()`, the `ConfigBuilder` API,
  runtime reconfiguration, and restricted `dictConfig()` and `fileConfig()`
  ([configure logging][guide-config]).
- **Rust integration**: bridges for the `log` and `tracing` crates, and
  versioned exception payloads ([integrate with Rust][guide-rust]).

______________________________________________________________________

## Status

Femtologging 0.2.0 is in beta. The logging runtime, handlers, configuration
APIs, and structured context are in place, and every example in this README and
the guides is executed by the test suite. Some standard library features, such
as `%`-style lazy formatting and named formatters, are not implemented yet, and
the public API may still change before a stable release. Pin the version you
depend on, and read the [changelog][changelog] and [migration guide][migration]
when you upgrade. Journald and OpenTelemetry integrations are planned; see the
[roadmap][roadmap].

______________________________________________________________________

## Learn more

- [Users' guide][guide] — from a first log message to production
  configuration and Rust integration
- [Changelog][changelog] — what changed in each release
- [0.2.0 migration guide][migration] — upgrading an existing application
- [Roadmap][roadmap] — planned features and progress
- [Developers' guide][developers] — building, testing, and maintaining
  femtologging
- [Design notes][design] — the architecture behind the Rust runtime, which
  ports core ideas from [picologging](https://github.com/microsoft/picologging)

______________________________________________________________________

## Licence

ISC — see [LICENSE][licence] for details.

______________________________________________________________________

## Contributing

Contributions are welcome! Start with the [developers' guide][developers] and
the [development workflow][workflow] for how to build and test the project;
automated contributors should also follow [AGENTS.md][agents].

<!-- This README is also the PyPI project description, where relative links do
not resolve, so every link to a repository file is absolute. -->

[guide]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md
[guide-differences]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#differences-from-the-standard-library
[guide-handlers]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#send-records-somewhere
[guide-socket]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#send-records-over-a-socket
[guide-context]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#add-context-to-records
[guide-exceptions]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#record-exceptions-and-call-stacks
[guide-python-handler]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#write-your-own-handler-in-python
[guide-config]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#configure-logging
[guide-rust]: https://github.com/leynos/femtologging/blob/main/docs/users-guide.md#integrate-with-rust
[changelog]: https://github.com/leynos/femtologging/blob/main/CHANGELOG.md
[migration]: https://github.com/leynos/femtologging/blob/main/docs/v0-2-0-migration-guide.md
[roadmap]: https://github.com/leynos/femtologging/blob/main/docs/roadmap.md
[developers]: https://github.com/leynos/femtologging/blob/main/docs/developers-guide.md
[workflow]: https://github.com/leynos/femtologging/blob/main/docs/dev-workflow.md
[design]: https://github.com/leynos/femtologging/blob/main/docs/rust-multithreaded-logging-framework-for-python-design.md
[licence]: https://github.com/leynos/femtologging/blob/main/LICENSE
[agents]: https://github.com/leynos/femtologging/blob/main/AGENTS.md
