# Changelog

## [0.2.0-beta1] - 2026-11-11

_First pre-release of 0.2.0. Applications upgrading from 0.1 should follow the
[0.2.0 migration guide](docs/v0-2-0-migration-guide.md)._

### Changed

- **Breaking:** limit logger names to 255 UTF-8 bytes and 32 dot-separated
  segments, reject control characters, and cap the registry at 10,000 named
  loggers ([#544](https://github.com/leynos/femtologging/pull/544)).
- **Breaking:** replace per-logger worker threads with a fixed pool of one to
  four shared dispatch workers, so `threading.local` state in a handler may be
  shared between loggers
  ([#544](https://github.com/leynos/femtologging/pull/544)).
- **Breaking:** disable loggers omitted by a `disable_existing_loggers`
  configuration, so they reject records instead of still propagating to
  ancestors ([#547](https://github.com/leynos/femtologging/pull/547)).
- **Breaking:** honour the wrapped handler's `level` in `StdlibHandlerAdapter`
  ([#546](https://github.com/leynos/femtologging/pull/546)).
- **Breaking:** reject runtime handler and filter definitions that no logger
  references, and release definitions a change leaves unreferenced
  ([#548](https://github.com/leynos/femtologging/pull/548)).
- **Breaking:** build wheels and the sdist without the `test-util` feature,
  which removes the timed-rotation test hooks from them
  ([#552](https://github.com/leynos/femtologging/pull/552)).
- **Breaking:** return `PyResult` from the Rust `FemtoLogger::new()` and
  `Result` from `FemtoLogger::with_parent()`
  ([#544](https://github.com/leynos/femtologging/pull/544)).
- **Breaking:** declare `BasicConfig` as a slotted dataclass, so assigning an
  attribute other than its five fields raises `AttributeError`
  ([#428](https://github.com/leynos/femtologging/pull/428)).
- **Breaking:** apply `ConfigBuilder.build_and_init()`, `dictConfig()`, and
  `fileConfig()` all or nothing, leaving the previous configuration in place
  when a call raises ([#428](https://github.com/leynos/femtologging/pull/428)).
- **Breaking:** run Python handlers, including `StdlibHandlerAdapter`, inside a
  copy of the caller's `contextvars` context
  ([#425](https://github.com/leynos/femtologging/pull/425)).
- **Breaking:** bound `FemtoHTTPHandler.close()` by the handler's write timeout
  instead of waiting for every retry
  ([#484](https://github.com/leynos/femtologging/pull/484)).
- **Breaking:** return `Result<(), HandlerError>` from the Rust
  `FemtoLogger::add_handler()` and reject Python-backed handlers that lack
  contextual dispatch with `HandlerError::MissingContextDispatch`
  ([#425](https://github.com/leynos/femtologging/pull/425)).
- Decide Rust `log::log_enabled!` per target from femtologging's current levels
  ([#545](https://github.com/leynos/femtologging/pull/545)).
- Mark the package as `Development Status :: 4 - Beta` and publish its home
  page, repository, and documentation URLs
  ([#543](https://github.com/leynos/femtologging/pull/543)).

### Added

- Add `with_filters()` to handler builders, and handler `filters` in
  `dictConfig()`, for filters that also run on propagated records
  ([#426](https://github.com/leynos/femtologging/pull/426)).
- Implement `FormatterBuilder` with `%`-style format strings, usable directly or
  by name through `ConfigBuilder` and `dictConfig()`
  ([#426](https://github.com/leynos/femtologging/pull/426)).
- Add `FemtoLogger.disabled` and `get_disabled_drops()`
  ([#547](https://github.com/leynos/femtologging/pull/547)).
- Add the `max_exception_depth` and `max_exception_nodes` limits to
  `filter_frames()`, which now rejects cyclic payloads
  ([#553](https://github.com/leynos/femtologging/pull/553)).
- Publish an attested sdist and native wheels to PyPI and GitHub Releases
  ([#552](https://github.com/leynos/femtologging/pull/552)).
- Accept a keyword-only `extra` mapping of structured fields on every logging
  method except `exception()`
  ([#361](https://github.com/leynos/femtologging/pull/361), [#424](https://github.com/leynos/femtologging/pull/424)).
- Add `log_context()` for scoped fields that follow `asyncio` tasks
  ([#424](https://github.com/leynos/femtologging/pull/424), [#454](https://github.com/leynos/femtologging/pull/454)).
- Add `PythonCallbackFilterBuilder` for callable and standard-library-style
  filters, with `"()"` factory support in `dictConfig()`
  ([#366](https://github.com/leynos/femtologging/pull/366)).
- Add `FemtoTimedRotatingFileHandler` and `TimedRotatingFileHandlerBuilder`
  for time-based rotation, including `dictConfig()` and `fileConfig()` support
  ([#362](https://github.com/leynos/femtologging/pull/362)).
- Add `RuntimeConfigBuilder` and `LoggerMutationBuilder` to change handlers,
  filters, levels, and propagation on running loggers in one transactional step
  ([#363](https://github.com/leynos/femtologging/pull/363)).
- Add the `get_context_dropped()` and `get_context_capture_failures()` logger
  counters
  ([#425](https://github.com/leynos/femtologging/pull/425), [#454](https://github.com/leynos/femtologging/pull/454)).
- Rewrite the users' guide as a path from a first message to production, and
  test every example in the README and guides
  ([#543](https://github.com/leynos/femtologging/pull/543)).

### Fixed

- Release the GIL while `flush_handlers()` and each built-in handler's `flush()`
  and `close()` wait on worker threads, which previously stalled, could lose
  queued records, and froze other Python threads
  ([#546](https://github.com/leynos/femtologging/pull/546), [#592](https://github.com/leynos/femtologging/pull/592)).

[0.2.0-beta1]: https://github.com/leynos/femtologging/releases/tag/v0.2.0-beta1
