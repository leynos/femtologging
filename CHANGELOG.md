# Changelog

## [0.2.0-beta1] - 2026-10-24

_First pre-release of 0.2.0. Applications upgrading from 0.1 should follow the
[0.2.0 migration guide](docs/v0-2-0-migration-guide.md)._

### Changed

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
- Mark the package as `Development Status :: 4 - Beta` and publish its home
  page, repository, and documentation URLs
  ([#543](https://github.com/leynos/femtologging/pull/543)).

### Added

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

- Release the GIL while `flush_handlers()`, `flush()`, and `close()` wait on
  worker threads, which previously stalled and could lose queued records
  ([#543](https://github.com/leynos/femtologging/pull/543)).

[0.2.0-beta1]: https://github.com/leynos/femtologging/releases/tag/v0.2.0-beta1
